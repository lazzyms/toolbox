use std::{collections::{BTreeMap, HashMap, HashSet}, fs, io::Write, path::{Path, PathBuf}, process::Command, sync::atomic::{AtomicU64, Ordering}, time::{Instant}};
use base64::Engine;
use rustybuzz::ttf_parser;
use flate2::{write::ZlibEncoder, Compression};
use lopdf::{dictionary, Document, Object, ObjectId, Stream};
use quick_xml::{events::Event, escape::unescape, Reader as XmlReader, XmlVersion};
use serde::{Deserialize, Serialize};
use crate::kit::{common::{batch_runner::report_progress, limits, note_renamed, watermark::{WatermarkOptions, WatermarkPlacement, WatermarkPosition}, JobOutcome, OutputLocation, OutputNaming}, contracts::{LimitKind, OutcomePointer, ToolError}};
use super::metadata::{PdfDocumentMetadata, PdfPageMetadata, PdfTextRun};
use super::{inherited, resolve, text_edit, PdfMutationIntent};

mod forms;
pub use forms::{inspect_form, FormInspection};

#[derive(Clone, Debug, Deserialize)]
pub struct Rect { pub x: f32, pub y: f32, pub width: f32, pub height: f32 }
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all="camelCase")]
pub struct SceneObject {
    pub kind: Kind, pub rect: Rect, pub text: String, pub font_size: f32,
    pub color: String, pub opacity: f32,
    #[serde(default)] pub shape: Option<Shape>,
    #[serde(default)] pub highlight_mode: Option<HighlightMode>,
    #[serde(default)] pub signature_mode: Option<SignatureMode>,
    #[serde(default)] pub signature_path: Option<PathBuf>,
    #[serde(default)] pub font_family: Option<String>,
    #[serde(default)] pub watermark: Option<WatermarkOptions>,
    #[serde(default)] pub layer: Layer,
    #[serde(default)] pub bold: bool,
    #[serde(default)] pub italic: bool,
    #[serde(default)] pub align: TextAlign,
    #[serde(default)] pub page_range: Option<String>,
}
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all="lowercase")]
pub enum TextAlign { #[default] Left, Center, Right }
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all="camelCase")]
pub enum Kind { Text, Highlight, Shape, Signature, Watermark }
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all="kebab-case")]
pub enum Shape { Square, Round, Triangle, Line, DottedLine, ArrowLeft, ArrowRight, ArrowUp, ArrowDown }
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all="kebab-case")]
pub enum HighlightMode { Area, TextSelection }
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all="kebab-case")]
pub enum SignatureMode { Image, Text }
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all="lowercase")]
pub enum Layer { #[default] Front, Behind }
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all="camelCase")]
pub struct ScenePage {
    pub source_index: Option<usize>, pub width: f32, pub height: f32,
    pub rotation: i32, pub crop: Option<Rect>,
    #[serde(default)] pub source_rotation: Option<i32>,
    #[serde(default)] pub source_box: Option<Rect>,
    pub objects: Vec<SceneObject>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct PdfScene { pub pages: Vec<ScenePage> }
#[derive(Deserialize)]
#[serde(rename_all="camelCase")]
pub struct PreviewRequest { pub path: PathBuf, pub scene: PdfScene, pub page_index: usize, #[serde(flatten)] pub sections: StageSections }
#[derive(Deserialize)]
#[serde(rename_all="camelCase")]
pub struct PreviewPagesRequest { pub path: PathBuf, pub scene: PdfScene, pub page_indices: Vec<usize>, #[serde(flatten)] pub sections: StageSections }
#[derive(Deserialize)]
#[serde(rename_all="camelCase")]
pub struct ExportRequest {
    pub paths: Vec<PathBuf>, pub scene: PdfScene, pub output_location: OutputLocation,
    /// Export As: the file the person chose in the save step. Never replaced; a taken name gets "-1", "-2".
    #[serde(default)] pub output_path: Option<PathBuf>,
    #[serde(flatten)] pub sections: StageSections,
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all="camelCase")]
pub struct StageSections {
    pub form_values: Option<Vec<serde_json::Value>>,
    pub text_edits: Option<Vec<text_edit::TextEdit>>,
    pub flatten: Option<bool>,
    pub redactions: Option<Vec<RedactionMark>>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all="camelCase")]
pub struct RedactionMark { pub page_index: usize, pub rect: Rect, pub color: String }
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct ScenePreview { pub data_url: String, pub width: u32, pub height: u32 }
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct IndexedScenePreview { pub page_index: usize, pub preview: ScenePreview }

fn err(e: impl std::fmt::Display) -> String { e.to_string() }
fn bounds(doc: &Document, v: &Object) -> Result<[f32;4], String> {
    let a = resolve(doc,v)?.as_array().map_err(err)?;
    if a.len()!=4 { return Err("Invalid page box".into()); }
    let mut b=[0.;4];
    for i in 0..4 { b[i]=super::metadata::number(resolve(doc,&a[i])?)?; }
    if b.iter().any(|n| !n.is_finite()) || b[2]<=b[0] || b[3]<=b[1] { return Err("Invalid page box".into()); }
    Ok(b)
}
struct Geometry { bbox: [f32;4], matrix: [f32;6], width: f32, height: f32 }
fn geometry(doc: &Document, id: ObjectId) -> Result<Geometry,String> {
    let media = bounds(doc,&inherited(doc,id,b"MediaBox")?.ok_or("Missing MediaBox")?)?;
    let mut b = match inherited(doc,id,b"CropBox")? { Some(v)=>bounds(doc,&v)?, None=>media };
    b=[b[0].max(media[0]),b[1].max(media[1]),b[2].min(media[2]),b[3].min(media[3])];
    let (w,h)=(b[2]-b[0],b[3]-b[1]);
    dimensions(w,h)?;
    let rotate = inherited(doc,id,b"Rotate")?.map(|v| v.as_i64().map_err(err)).transpose()?.unwrap_or(0);
    if rotate%90!=0 { return Err("Source rotation must be a multiple of 90".into()); }
    // Source PDF bottom-left coordinates to visible, top-left scene coordinates.
    let (matrix,width,height)=match rotate.rem_euclid(360) {
        0=>([1.,0.,0.,-1.,-b[0],b[3]],w,h),
        90=>([0.,1.,1.,0.,-b[1],-b[0]],h,w),
        180=>([-1.,0.,0.,1.,b[2],-b[1]],w,h),
        _=>([0.,-1.,-1.,0.,b[3],b[2]],h,w),
    };
    if let Some(unit)=doc.get_dictionary(id).map_err(err)?.get(b"UserUnit").ok() {
        if super::metadata::number(resolve(doc,unit)?)? != 1. { return Err("PDF UserUnit other than 1 is not supported".into()); }
    }
    Ok(Geometry{bbox:b,matrix,width,height})
}
fn dimensions(w:f32,h:f32)->Result<(),String> {
    if !w.is_finite() || !h.is_finite() || w<=0. || h<=0. || w>14400. || h>14400. { Err("Page dimensions must be within 0–14400 points".into()) } else { Ok(()) }
}
fn valid_rect(r:&Rect)->Result<(),String> {
    dimensions(r.width,r.height)?;
    if !r.x.is_finite() || !r.y.is_finite() || r.x.abs()>14400. || r.y.abs()>14400. { return Err("Invalid rectangle origin".into()); } Ok(())
}
/// Why a password-protected PDF cannot be opened or exported, with the next step. The editor never asks for passwords;
/// File Security removes them into a new copy.
pub const LOCKED_PDF_MESSAGE:&str="This PDF is password-protected. Remove its password with File Security first, then open the unlocked copy here.";
/// The typed refusal for a password-protected PDF: invalid input that points to File Security.
fn locked_pdf()->ToolError { ToolError::invalid_input(LOCKED_PDF_MESSAGE).with_pointer(OutcomePointer::Tool{tool_id:"lock-unlock".into()}) }
fn load(path:&Path)->Result<Document,String> {
    let d=Document::load(path).map_err(err)?;
    if d.is_encrypted() { return Err(LOCKED_PDF_MESSAGE.into()); } Ok(d)
}
fn cm(m:[f32;6])->String { format!("{} {} {} {} {} {} cm\n",m[0],m[1],m[2],m[3],m[4],m[5]) }
fn array(v:&[f32])->Object { Object::Array(v.iter().copied().map(Object::Real).collect()) }

fn rgb(color: &str) -> Result<[f32; 3], String> {
    let hex = color.strip_prefix('#').ok_or("Color must be #RRGGBB")?;
    if hex.len() != 6 || !hex.is_ascii() { return Err("Color must be #RRGGBB".into()); }
    Ok([
        u8::from_str_radix(&hex[0..2], 16).map_err(err)? as f32 / 255.,
        u8::from_str_radix(&hex[2..4], 16).map_err(err)? as f32 / 255.,
        u8::from_str_radix(&hex[4..6], 16).map_err(err)? as f32 / 255.,
    ])
}

const SIGNATURE_FONT: &[u8] = include_bytes!("../../../resources/fonts/Pacifico-Regular.ttf");

const TEXT_MAX_CHARACTERS: usize = 100_000;
const TEXT_LINE_HEIGHT: f32 = 1.2;
const SYNTHETIC_OBLIQUE: f32 = 0.2;
const NOTO_SANS_REGULAR: &[u8] = include_bytes!("../../../resources/fonts/NotoSans-Regular.ttf");
const NOTO_SANS_BOLD: &[u8] = include_bytes!("../../../resources/fonts/NotoSans-Bold.ttf");
const NOTO_SANS_ITALIC: &[u8] = include_bytes!("../../../resources/fonts/NotoSans-Italic.ttf");
const NOTO_SANS_BOLD_ITALIC: &[u8] = include_bytes!("../../../resources/fonts/NotoSans-BoldItalic.ttf");
const NOTO_SERIF_REGULAR: &[u8] = include_bytes!("../../../resources/fonts/NotoSerif-Regular.ttf");
const NOTO_SERIF_BOLD: &[u8] = include_bytes!("../../../resources/fonts/NotoSerif-Bold.ttf");
const NOTO_SERIF_ITALIC: &[u8] = include_bytes!("../../../resources/fonts/NotoSerif-Italic.ttf");
const NOTO_SERIF_BOLD_ITALIC: &[u8] = include_bytes!("../../../resources/fonts/NotoSerif-BoldItalic.ttf");
const NOTO_SANS_MONO_REGULAR: &[u8] = include_bytes!("../../../resources/fonts/NotoSansMono-Regular.ttf");
const NOTO_SANS_MONO_BOLD: &[u8] = include_bytes!("../../../resources/fonts/NotoSansMono-Bold.ttf");

#[derive(Clone, Copy)]
struct TextFont { family: &'static str, name: &'static str, bytes: &'static [u8], oblique: bool }

fn text_font(o: &SceneObject) -> Result<TextFont, ToolError> {
    let family = o.font_family.as_deref().unwrap_or("Noto Sans");
    let (name, bytes) = match (family, o.bold, o.italic) {
        ("Noto Sans", false, false) => ("NotoSans-Regular", NOTO_SANS_REGULAR),
        ("Noto Sans", true, false) => ("NotoSans-Bold", NOTO_SANS_BOLD),
        ("Noto Sans", false, true) => ("NotoSans-Italic", NOTO_SANS_ITALIC),
        ("Noto Sans", true, true) => ("NotoSans-BoldItalic", NOTO_SANS_BOLD_ITALIC),
        ("Noto Serif", false, false) => ("NotoSerif-Regular", NOTO_SERIF_REGULAR),
        ("Noto Serif", true, false) => ("NotoSerif-Bold", NOTO_SERIF_BOLD),
        ("Noto Serif", false, true) => ("NotoSerif-Italic", NOTO_SERIF_ITALIC),
        ("Noto Serif", true, true) => ("NotoSerif-BoldItalic", NOTO_SERIF_BOLD_ITALIC),
        // Noto Sans Mono has no italic; italic is drawn with a slanted text matrix.
        ("Noto Sans Mono", false, _) => ("NotoSansMono-Regular", NOTO_SANS_MONO_REGULAR),
        ("Noto Sans Mono", true, _) => ("NotoSansMono-Bold", NOTO_SANS_MONO_BOLD),
        _ => return Err(ToolError::invalid_input(format!("{family} is not a bundled text font. Choose Noto Sans, Noto Serif or Noto Sans Mono."))),
    };
    let family = match family { "Noto Serif" => "Noto Serif", "Noto Sans Mono" => "Noto Sans Mono", _ => "Noto Sans" };
    Ok(TextFont { family, name, bytes, oblique: o.italic && family == "Noto Sans Mono" })
}

fn parse_face(bytes: &[u8]) -> Result<ttf_parser::Face<'_>, ToolError> {
    ttf_parser::Face::parse(bytes, 0).map_err(|error| ToolError::processing(format!("Could not read a bundled text font: {error}")))
}

fn glyph_width(face: &ttf_parser::Face<'_>, gid: u16) -> i64 {
    let advance = face.glyph_hor_advance(ttf_parser::GlyphId(gid)).unwrap_or(0) as f32;
    (advance * 1000. / face.units_per_em() as f32).round() as i64
}

struct TextLine { glyphs: Vec<(u16, char)>, width: f32 }
struct TextLayout { font: TextFont, lines: Vec<TextLine>, first_baseline: f32 }

fn unrenderable_message(family: &str, missing: &[char], subject: &str) -> String {
    let named = missing.iter().take(10).map(|c| format!("“{c}” (U+{:04X})", *c as u32)).collect::<Vec<_>>().join(", ");
    let more = if missing.len() > 10 { format!(" and {} more", missing.len() - 10) } else { String::new() };
    let noun = if missing.len() == 1 { "this character" } else { "these characters" };
    format!("{family} can't show {noun}: {named}{more}. Remove {} to export {subject}.", if missing.len() == 1 { "it" } else { "them" })
}

fn layout_text(o: &SceneObject) -> Result<TextLayout, ToolError> { layout_text_for(o, "this text box") }

fn layout_text_for(o: &SceneObject, subject: &str) -> Result<TextLayout, ToolError> {
    layout_lines(text_font(o)?, &o.text, o.font_size, o.rect.width + 0.01, subject)
}

fn layout_lines(font: TextFont, text: &str, font_size: f32, max_width: f32, subject: &str) -> Result<TextLayout, ToolError> {
    if text.chars().count() > TEXT_MAX_CHARACTERS {
        return Err(ToolError::limit_exceeded(LimitKind::TextLength,
            "Text box text is longer than the 100,000-character limit. Shorten it and try again."));
    }
    let face = parse_face(font.bytes)?;
    let mut missing = Vec::new();
    let mut glyph = |c: char| match face.glyph_index(c) {
        Some(id) if id.0 != 0 => Some(id.0),
        _ => { if !missing.contains(&c) { missing.push(c); } None }
    };
    let scale = font_size / 1000.;
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let paragraph = paragraph.strip_suffix('\r').unwrap_or(paragraph);
        let mut line: Vec<(u16, char, f32)> = Vec::new();
        let mut break_after = None;
        let finish = |line: Vec<(u16, char, f32)>, lines: &mut Vec<TextLine>| {
            let end = line.iter().rposition(|(_, c, _)| *c != ' ').map_or(0, |index| index + 1);
            let glyphs: Vec<_> = line[..end].iter().map(|(gid, c, _)| (*gid, *c)).collect();
            let width = line[..end].iter().map(|(_, _, width)| width).sum();
            lines.push(TextLine { glyphs, width });
        };
        for c in paragraph.chars() {
            let Some(gid) = glyph(c) else { continue };
            let width = glyph_width(&face, gid) as f32 * scale;
            let used: f32 = line.iter().map(|(_, _, width)| width).sum();
            if c != ' ' && used + width > max_width && line.iter().any(|(_, c, _)| *c != ' ') {
                let carry = match break_after.take() { Some(index) => line.split_off(index), None => Vec::new() };
                finish(std::mem::replace(&mut line, carry), &mut lines);
                let used: f32 = line.iter().map(|(_, _, width)| width).sum();
                if used + width > max_width && !line.is_empty() { finish(std::mem::take(&mut line), &mut lines); }
            }
            line.push((gid, c, width));
            if c == ' ' { break_after = Some(line.len()); }
        }
        finish(line, &mut lines);
    }
    if !missing.is_empty() { return Err(ToolError::invalid_input(unrenderable_message(font.family, &missing, subject))); }
    let em = face.units_per_em() as f32;
    let (ascent, descent) = (face.ascender() as f32 / em, face.descender() as f32 / em);
    // Matches CSS line-height: half-leading above the ascender.
    let first_baseline = font_size * ((TEXT_LINE_HEIGHT - (ascent - descent)) / 2. + ascent);
    Ok(TextLayout { font, lines, first_baseline })
}

fn page_range_pages(range: &str, page_count: usize) -> Result<Vec<usize>, ToolError> {
    let invalid = || ToolError::invalid_input(format!("“{range}” is not a page list. Use page numbers and ranges like 1-3, 7, 10-, or all, odd or even."));
    let number = |value: &str| value.trim().parse::<usize>().ok().filter(|value| *value > 0).ok_or_else(invalid);
    let mut pages = std::collections::BTreeSet::new();
    for item in range.split(',') {
        let item = item.trim();
        if item.is_empty() { return Err(invalid()); }
        if let Some((first, step)) = match item.to_ascii_lowercase().as_str() { "all" => Some((1, 1)), "odd" => Some((1, 2)), "even" => Some((2, 2)), _ => None } {
            pages.extend((first..=page_count).step_by(step).map(|page| page - 1));
            continue;
        }
        let (first, last) = match item.split_once('-') {
            Some((first, "")) => (number(first)?, usize::MAX),
            Some((first, last)) => (number(first)?, number(last)?),
            None => { let page = number(item)?; (page, page) }
        };
        if first > last { return Err(invalid()); }
        pages.extend((first..=last.min(page_count)).map(|page| page - 1));
    }
    if pages.is_empty() {
        return Err(ToolError::invalid_input(format!("Pages {range} are not in this document. It has {page_count} page{}.", if page_count == 1 { "" } else { "s" })));
    }
    Ok(pages.into_iter().collect())
}

struct TextPlan { placed: Vec<Vec<(usize, usize)>>, layouts: HashMap<(usize, usize), TextLayout>, signatures: HashMap<(usize, usize), SignatureImage> }

fn text_plan(scene: &PdfScene) -> Result<TextPlan, ToolError> {
    let mut placed = vec![Vec::new(); scene.pages.len()];
    let mut layouts = HashMap::new();
    let mut signatures = HashMap::new();
    for (origin, page) in scene.pages.iter().enumerate() {
        for (index, o) in page.objects.iter().enumerate() {
            match (&o.kind, o.page_range.as_deref()) {
                (Kind::Text | Kind::Signature | Kind::Watermark, Some(range)) => for target in page_range_pages(range, scene.pages.len())? { placed[target].push((origin, index)); },
                (_, Some(_)) => return Err(ToolError::invalid_input("Only text boxes, signatures and watermarks can be shown on several pages")),
                _ => placed[origin].push((origin, index)),
            }
            match o.kind {
                Kind::Text => { layouts.insert((origin, index), layout_text(o)?); }
                Kind::Signature => { signatures.insert((origin, index), signature(o)?); }
                Kind::Watermark => {
                    let options = watermark_options(o)?;
                    if let Some(path) = &options.logo_path { check_watermark_image(path)?; }
                    // Laid out at 1 pt without wrapping; each page scales it to its own size.
                    if let Some(text) = options.text() { layouts.insert((origin, index), layout_lines(text_font(o)?, text, 1., f32::INFINITY, "this watermark")?); }
                }
                _ => {}
            }
        }
    }
    Ok(TextPlan { placed, layouts, signatures })
}

fn hex_utf16(c: char) -> String {
    c.encode_utf16(&mut [0; 2]).iter().map(|unit| format!("{unit:04X}")).collect()
}

fn subset_tag(gids: &[u16]) -> String {
    let mut hash: u32 = 0x811c9dc5;
    for byte in gids.iter().flat_map(|gid| gid.to_be_bytes()) { hash = (hash ^ byte as u32).wrapping_mul(0x01000193); }
    (0..6).map(|index| (b'A' + ((hash >> (index * 5)) % 26) as u8) as char).collect()
}

/// Embeds one subset Type0 font per bundled face and returns the new glyph id of each source glyph.
fn embed_text_fonts(doc: &mut Document, layouts: &HashMap<(usize, usize), TextLayout>) -> Result<HashMap<&'static str, (ObjectId, subsetter::GlyphRemapper)>, String> {
    let mut used: std::collections::BTreeMap<&'static str, (TextFont, std::collections::BTreeMap<u16, char>)> = Default::default();
    let mut keys: Vec<_> = layouts.keys().copied().collect();
    keys.sort();
    for key in keys {
        let layout = &layouts[&key];
        let entry = used.entry(layout.font.name).or_insert_with(|| (layout.font, Default::default()));
        for (gid, c) in layout.lines.iter().flat_map(|line| &line.glyphs) { entry.1.entry(*gid).or_insert(*c); }
    }
    let mut fonts = HashMap::new();
    for (name, (font, glyphs)) in used {
        let face = parse_face(font.bytes).map_err(|error| error.message)?;
        let gids: Vec<u16> = glyphs.keys().copied().collect();
        let remapper = subsetter::GlyphRemapper::new_from_glyphs_sorted(&gids);
        let subset = subsetter::subset(font.bytes, 0, &remapper).map_err(|error| format!("Could not subset {name}: {error}"))?;
        let base_font = format!("{}+{name}", subset_tag(&gids));
        let em = face.units_per_em() as f32;
        let units = |value: i16| (value as f32 * 1000. / em).round() as i64;
        let bbox = face.global_bounding_box();
        let length = subset.len() as i64;
        let file = doc.add_object(Stream::new(dictionary! { "Length1" => length, "Filter" => "FlateDecode" }, compress_bytes(&subset)?));
        let italic = name.contains("Italic");
        let flags = 32 | if face.is_monospaced() { 1 } else { 0 } | if italic { 64 } else { 0 };
        let descriptor = doc.add_object(dictionary! {
            "Type" => "FontDescriptor", "FontName" => base_font.as_str(), "Flags" => flags,
            "FontBBox" => vec![units(bbox.x_min).into(), units(bbox.y_min).into(), units(bbox.x_max).into(), units(bbox.y_max).into()],
            "ItalicAngle" => face.italic_angle(), "Ascent" => units(face.ascender()), "Descent" => units(face.descender()),
            "CapHeight" => units(face.capital_height().unwrap_or(face.ascender())), "StemV" => if name.contains("Bold") { 120 } else { 80 },
            "FontFile2" => file,
        });
        let widths: Vec<Object> = gids.iter().map(|gid| Object::Integer(glyph_width(&face, *gid))).collect();
        let cid_font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "CIDFontType2", "BaseFont" => base_font.as_str(),
            "CIDSystemInfo" => dictionary! { "Registry" => Object::string_literal("Adobe"), "Ordering" => Object::string_literal("Identity"), "Supplement" => 0 },
            "FontDescriptor" => descriptor, "CIDToGIDMap" => "Identity",
            "W" => vec![Object::Integer(1), Object::Array(widths)],
        });
        let mut cmap = String::from("/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n");
        let entries: Vec<_> = glyphs.iter().map(|(gid, c)| format!("<{:04X}> <{}>\n", remapper.get(*gid).unwrap_or(0), hex_utf16(*c))).collect();
        for chunk in entries.chunks(100) { cmap.push_str(&format!("{} beginbfchar\n{}endbfchar\n", chunk.len(), chunk.concat())); }
        cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
        let to_unicode = doc.add_object(Stream::new(dictionary! {}, cmap.into_bytes()));
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type0", "BaseFont" => base_font.as_str(), "Encoding" => "Identity-H",
            "DescendantFonts" => vec![Object::Reference(cid_font)], "ToUnicode" => to_unicode,
        });
        fonts.insert(name, (font_id, remapper));
    }
    Ok(fonts)
}

fn text_content(o: &SceneObject, layout: &TextLayout, resource: &str, remapper: &subsetter::GlyphRemapper) -> String {
    let r = &o.rect;
    let slant = if layout.font.oblique { SYNTHETIC_OBLIQUE } else { 0. };
    let mut content = format!("{} {} {} {} re W n\nBT /{resource} {} Tf\n", r.x, r.y, r.width, r.height, o.font_size);
    for (index, line) in layout.lines.iter().enumerate() {
        if line.glyphs.is_empty() { continue; }
        let x = r.x + match o.align { TextAlign::Left => 0., TextAlign::Center => (r.width - line.width) / 2., TextAlign::Right => r.width - line.width };
        let y = r.y + layout.first_baseline + index as f32 * o.font_size * TEXT_LINE_HEIGHT;
        let glyphs: String = line.glyphs.iter().map(|(gid, _)| format!("{:04X}", remapper.get(*gid).unwrap_or(0))).collect();
        content.push_str(&format!("1 0 {slant} -1 {x} {y} Tm <{glyphs}> Tj\n"));
    }
    content.push_str("ET\n");
    content
}

fn shape_content(shape: Option<&Shape>, r: &Rect) -> String {
    let shape = shape.unwrap_or(&Shape::Square);
    match shape {
        Shape::Square => format!("{} {} {} {} re S\n", r.x, r.y, r.width, r.height),
        Shape::Round => {
            let k = 0.55228475_f32;
            let rx = r.width / 2.; let ry = r.height / 2.;
            let cx = r.x + rx; let cy = r.y + ry;
            format!("{} {} m {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c S\n",
                cx + rx, cy,
                cx + rx, cy + k * ry, cx + k * rx, cy + ry, cx, cy + ry,
                cx - k * rx, cy + ry, cx - rx, cy + k * ry, cx - rx, cy,
                cx - rx, cy - k * ry, cx - k * rx, cy - ry, cx, cy - ry,
                cx + k * rx, cy - ry, cx + rx, cy - k * ry, cx + rx, cy)
        }
        Shape::Triangle => format!("{} {} m {} {} l {} {} l h S\n", r.x, r.y + r.height, r.x + r.width / 2., r.y, r.x + r.width, r.y + r.height),
        Shape::Line => format!("{} {} m {} {} l S\n", r.x, r.y + r.height / 2., r.x + r.width, r.y + r.height / 2.),
        Shape::DottedLine => format!("[3 4] 0 d {} {} m {} {} l S\n", r.x, r.y + r.height / 2., r.x + r.width, r.y + r.height / 2.),
        Shape::ArrowRight => format!("{} {} m {} {} l {} {} m {} {} l {} {} l h f\n",
            r.x, r.y + r.height / 2., r.x + r.width - 10., r.y + r.height / 2.,
            r.x + r.width, r.y + r.height / 2., r.x + r.width - 10., r.y + r.height / 2. - 7., r.x + r.width - 10., r.y + r.height / 2. + 7.),
        Shape::ArrowLeft => format!("{} {} m {} {} l {} {} m {} {} l {} {} l h f\n",
            r.x + 10., r.y + r.height / 2., r.x + r.width, r.y + r.height / 2.,
            r.x, r.y + r.height / 2., r.x + 10., r.y + r.height / 2. - 7., r.x + 10., r.y + r.height / 2. + 7.),
        Shape::ArrowDown => format!("{} {} m {} {} l {} {} m {} {} l {} {} l h f\n",
            r.x + r.width / 2., r.y, r.x + r.width / 2., r.y + r.height - 10.,
            r.x + r.width / 2., r.y + r.height, r.x + r.width / 2. - 7., r.y + r.height - 10., r.x + r.width / 2. + 7., r.y + r.height - 10.),
        Shape::ArrowUp => format!("{} {} m {} {} l {} {} m {} {} l {} {} l h f\n",
            r.x + r.width / 2., r.y + 10., r.x + r.width / 2., r.y + r.height,
            r.x + r.width / 2., r.y, r.x + r.width / 2. - 7., r.y + 10., r.x + r.width / 2. + 7., r.y + 10.),
    }
}

const WATERMARK_DEFAULT_SIZE: f32 = 0.1;
const WATERMARK_MAX_TILES: usize = 2000;
const WATERMARK_MAX_PIXELS: u64 = 20_000_000;

fn watermark_options(o: &SceneObject) -> Result<&WatermarkOptions, ToolError> {
    let options = o.watermark.as_ref().ok_or_else(|| ToolError::invalid_input("Add watermark text or choose a picture."))?;
    options.validate().map_err(ToolError::invalid_input)?;
    if options.text().is_some() && options.logo_path.is_some() { return Err(ToolError::invalid_input("Choose text or a picture for the watermark, not both.")); }
    Ok(options)
}

#[derive(Debug, Serialize)]
pub struct WatermarkImageInfo { pub width: u32, pub height: u32 }

/// Refuses a watermark picture before anything is placed: PNG or JPG within the image tools' recorded limits.
pub fn check_watermark_image(path: &Path) -> Result<WatermarkImageInfo, ToolError> {
    crate::kit::common::limits::recorded("watermark_images").ok_or_else(|| ToolError::processing("Image limits are missing."))?.check(path)?;
    let unreadable = |error: std::io::Error| ToolError::invalid_input(format!("Could not read the watermark picture: {error}"));
    let reader = image::ImageReader::open(path).map_err(unreadable)?.with_guessed_format().map_err(unreadable)?;
    if !matches!(reader.format(), Some(image::ImageFormat::Png | image::ImageFormat::Jpeg)) {
        return Err(ToolError::invalid_input("Choose a PNG or JPG picture for the watermark."));
    }
    let (width, height) = reader.into_dimensions().map_err(|error| ToolError::invalid_input(format!("Could not read the watermark picture: {error}")))?;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > WATERMARK_MAX_PIXELS {
        return Err(ToolError::limit_exceeded(LimitKind::Decompression, "This picture is larger than 20 megapixels, the limit for a watermark. Use a smaller picture."));
    }
    Ok(WatermarkImageInfo { width, height })
}

pub fn inspect_watermark_image(path: &Path) -> Result<WatermarkImageInfo, String> { check_watermark_image(path).map_err(|error| error.message) }

fn watermark_image(path: &Path) -> Result<SignatureImage, String> {
    check_watermark_image(path).map_err(|error| error.message)?;
    rgba_image(image::open(path).map_err(|error| format!("Could not read the watermark picture: {error}"))?.to_rgba8()).map_err(|error| error.message)
}

/// The unrotated size of one mark in points: a text layout laid out at 1 pt, or a picture's pixel size.
fn watermark_size(options: &WatermarkOptions, area: &Rect, layout: Option<&TextLayout>, picture: Option<(u32, u32)>) -> (f32, f32) {
    let shorter = area.width.min(area.height);
    match (layout, picture) {
        (Some(layout), _) => {
            let font_size = options.size.unwrap_or(WATERMARK_DEFAULT_SIZE) * shorter;
            let widest = layout.lines.iter().map(|line| line.width).fold(0., f32::max);
            (widest * font_size, layout.lines.len() as f32 * TEXT_LINE_HEIGHT * font_size)
        }
        (None, Some((width, height))) => {
            let (width, height) = (width as f32, height as f32);
            // Without a size, a picture keeps the legacy watermark's 180 × 100 pt box, never enlarged.
            let scale = match options.size { Some(size) => size * shorter / height, None => (180. / width).min(100. / height).min(1.) };
            (width * scale, height * scale)
        }
        (None, None) => (0., 0.),
    }
}

/// Centres of each mark on a page's visible area. Rotation is in degrees, clockwise on screen.
fn watermark_centers(area: &Rect, (width, height): (f32, f32), position: Option<WatermarkPosition>, rotation: f32) -> Result<Vec<(f32, f32)>, ToolError> {
    let (sin, cos) = rotation.to_radians().sin_cos();
    let (box_w, box_h) = (width * cos.abs() + height * sin.abs(), width * sin.abs() + height * cos.abs());
    let margin = 0.04 * area.width.min(area.height);
    let (left, top) = (area.x + margin + box_w / 2., area.y + margin + box_h / 2.);
    let (right, bottom) = (area.x + area.width - margin - box_w / 2., area.y + area.height - margin - box_h / 2.);
    let (cx, cy) = (area.x + area.width / 2., area.y + area.height / 2.);
    Ok(match position.unwrap_or(WatermarkPosition::Placement(WatermarkPlacement::Center)) {
        WatermarkPosition::Placement(WatermarkPlacement::Center) => vec![(cx, cy)],
        WatermarkPosition::Placement(WatermarkPlacement::TopLeft) => vec![(left, top)],
        WatermarkPosition::Placement(WatermarkPlacement::TopRight) => vec![(right, top)],
        WatermarkPosition::Placement(WatermarkPlacement::BottomLeft) => vec![(left, bottom)],
        WatermarkPosition::Placement(WatermarkPlacement::BottomRight) => vec![(right, bottom)],
        WatermarkPosition::Offset { x, y } => vec![(area.x + x as f32 + box_w / 2., area.y + y as f32 + box_h / 2.)],
        WatermarkPosition::Placement(WatermarkPlacement::Tile) => {
            let gap = height.max(1.);
            let (step_x, step_y) = (box_w + gap, box_h + gap);
            let (columns, rows) = ((area.width / 2. / step_x).ceil() as i64 + 1, (area.height / 2. / step_y).ceil() as i64 + 1);
            if ((2 * columns + 1) * (2 * rows + 1)) as usize > WATERMARK_MAX_TILES {
                return Err(ToolError::invalid_input("The watermark is too small to tile across this page. Make it larger."));
            }
            // A grid centred on the page; alternate rows shift by half a step.
            (-rows..=rows).flat_map(|row| (-columns..=columns).map(move |column| (
                cx + column as f32 * step_x + if row.rem_euclid(2) == 1 { step_x / 2. } else { 0. },
                cy + row as f32 * step_y,
            ))).collect()
        }
    })
}

fn watermark_rotation(rotation: f32, (cx, cy): (f32, f32)) -> String {
    let (sin, cos) = rotation.to_radians().sin_cos();
    let clean = |value: f32| if value.abs() < 1e-6 { 0. } else { value };
    format!("{} {} {} {} {cx} {cy} cm\n", clean(cos), clean(sin), clean(-sin), clean(cos))
}

fn watermark_text_content(options: &WatermarkOptions, size: (f32, f32), area: &Rect, layout: &TextLayout, resource: &str, remapper: &subsetter::GlyphRemapper) -> String {
    let font_size = options.size.unwrap_or(WATERMARK_DEFAULT_SIZE) * area.width.min(area.height);
    let slant = if layout.font.oblique { SYNTHETIC_OBLIQUE } else { 0. };
    let mut content = format!("BT /{resource} {font_size} Tf\n");
    for (index, line) in layout.lines.iter().enumerate() {
        if line.glyphs.is_empty() { continue; }
        let x = -line.width * font_size / 2.;
        let y = -size.1 / 2. + (layout.first_baseline + index as f32 * TEXT_LINE_HEIGHT) * font_size;
        let glyphs: String = line.glyphs.iter().map(|(gid, _)| format!("{:04X}", remapper.get(*gid).unwrap_or(0))).collect();
        content.push_str(&format!("1 0 {slant} -1 {x} {y} Tm <{glyphs}> Tj\n"));
    }
    content.push_str("ET\n");
    content
}

struct SignatureImage { rgb: Vec<u8>, alpha: Option<Vec<u8>>, width: u32, height: u32 }

fn compress_bytes(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes).map_err(err)?;
    encoder.finish().map_err(err)
}

fn signature(o: &SceneObject) -> Result<SignatureImage, ToolError> {
    match o.signature_mode {
        Some(SignatureMode::Image) => signature_image(o.signature_path.as_deref().ok_or_else(|| ToolError::invalid_input("Choose a signature picture before placing it."))?),
        Some(SignatureMode::Text) => signature_text_image(&o.text, rgb(&o.color).map_err(ToolError::invalid_input)?),
        None => Err(ToolError::invalid_input("Type a name or choose a signature picture before placing it.")),
    }
}

#[derive(Debug, Serialize)]
pub struct SignatureSize { pub width: u32, pub height: u32 }

/// Applies the export-time signature checks when a picture is chosen, so refusals happen before placement.
pub fn inspect_signature_image(path: &Path) -> Result<SignatureSize, String> {
    signature_image(path).map(|image| SignatureSize { width: image.width, height: image.height }).map_err(|error| error.message)
}

fn signature_image(path: &Path) -> Result<SignatureImage, ToolError> {
    limits::SIGNATURE_IMAGE.check(path)?;
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| "This file".into());
    let refused = || ToolError::invalid_input(format!("{name} can't be used. Choose a PNG or JPG. A transparent PNG looks best."));
    let bytes = fs::read(path).map_err(|_| ToolError::invalid_input(format!("{name} couldn't be opened. Check that it still exists and try again.")))?;
    let format = image::guess_format(&bytes).ok().filter(|format| matches!(format, image::ImageFormat::Png | image::ImageFormat::Jpeg)).ok_or_else(refused)?;
    let image = image::load_from_memory_with_format(&bytes, format).map_err(|_| ToolError::invalid_input(format!("{name} couldn't be read. It may be damaged. Choose another PNG or JPG.")))?.to_rgba8();
    rgba_image(image)
}

fn rgba_image(image: image::RgbaImage) -> Result<SignatureImage, ToolError> {
    let mut rgb = Vec::with_capacity((image.width() * image.height() * 3) as usize);
    let mut alpha = Vec::with_capacity((image.width() * image.height()) as usize);
    for pixel in image.pixels() {
        rgb.extend_from_slice(&pixel.0[..3]);
        alpha.push(pixel.0[3]);
    }
    let alpha = alpha.iter().any(|value| *value < 255).then_some(alpha);
    let compress = |bytes: &[u8]| compress_bytes(bytes).map_err(ToolError::processing);
    Ok(SignatureImage { rgb: compress(&rgb)?, alpha: alpha.map(|values| compress(&values)).transpose()?, width: image.width(), height: image.height() })
}

/// Rasterizes typed signatures in the bundled script font, falling back to Noto Sans per glyph.
fn signature_text_image(text: &str, color: [f32; 3]) -> Result<SignatureImage, ToolError> {
    if text.trim().is_empty() { return Err(ToolError::invalid_input("Type a name for the signature before placing it.")); }
    if text.chars().count() > 200 { return Err(ToolError::invalid_input("Signatures can be up to 200 characters long. Shorten the name and try again.")); }
    let parse = |bytes: &'static [u8]| fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()).map_err(|error| ToolError::processing(format!("Could not load the signature font: {error}")));
    let fonts = [parse(SIGNATURE_FONT)?, parse(NOTO_SANS_REGULAR)?];
    let mut missing = Vec::new();
    for c in text.chars().filter(|c| *c != '\n') {
        if !fonts.iter().any(|font| font.has_glyph(c)) && !missing.contains(&c) { missing.push(c); }
    }
    if !missing.is_empty() { return Err(ToolError::invalid_input(unrenderable_message("The signature font", &missing, "this signature"))); }
    let font_for = |c: char| fonts.iter().find(|font| font.has_glyph(c)).unwrap_or(&fonts[0]);
    let raster_size = 96.0;
    let line_metrics = fonts[0].horizontal_line_metrics(raster_size).ok_or_else(|| ToolError::processing("Could not measure the signature font"))?;
    let line_height = (line_metrics.ascent - line_metrics.descent).max(raster_size * 1.1);
    let lines = text.split('\n').collect::<Vec<_>>();
    let line_widths = lines.iter().map(|line| line.chars().map(|c| font_for(c).metrics(c, raster_size).advance_width).sum::<f32>()).collect::<Vec<_>>();
    let width = (line_widths.iter().copied().fold(0.0, f32::max) + 16.0).ceil() as usize;
    let height = (line_height * lines.len() as f32 + 16.0).ceil() as usize;
    let pixels = width * height;
    let color = color.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8);
    let mut alpha = vec![0_u8; pixels];
    for (line_index, line) in lines.iter().enumerate() {
        let baseline = 8.0 + line_index as f32 * line_height + line_metrics.ascent;
        let mut pen_x = 8.0;
        for character in line.chars() {
            let (metrics, bitmap) = font_for(character).rasterize(character, raster_size);
            let glyph_x = (pen_x + metrics.xmin as f32).floor() as i64;
            let glyph_y = (baseline - metrics.height as f32 - metrics.ymin as f32).floor() as i64;
            for row in 0..metrics.height {
                for column in 0..metrics.width {
                    let value = bitmap[row * metrics.width + column];
                    if value == 0 { continue; }
                    let x = glyph_x + column as i64;
                    let y = glyph_y + row as i64;
                    if x < 0 || y < 0 || x >= width as i64 || y >= height as i64 { continue; }
                    let index = y as usize * width + x as usize;
                    alpha[index] = alpha[index].max(value);
                }
            }
            pen_x += metrics.advance_width;
        }
    }
    let mut rgb = Vec::with_capacity(pixels * 3);
    for _ in 0..pixels { rgb.extend_from_slice(&color); }
    let compress = |bytes: &[u8]| compress_bytes(bytes).map_err(ToolError::processing);
    Ok(SignatureImage { rgb: compress(&rgb)?, alpha: Some(compress(&alpha)?), width: width as u32, height: height as u32 })
}

fn add_signature_image(document: &mut Document, image: &SignatureImage) -> ObjectId {
    let mask = image.alpha.clone().map(|alpha| document.add_object(Object::Stream(Stream::new(dictionary! {
        "Type" => "XObject", "Subtype" => "Image", "Width" => image.width as i64,
        "Height" => image.height as i64, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
        "Filter" => "FlateDecode",
    }, alpha))));
    let mut properties = dictionary! {
        "Type" => "XObject", "Subtype" => "Image", "Width" => image.width as i64,
        "Height" => image.height as i64, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8,
        "Filter" => "FlateDecode",
    };
    if let Some(mask) = mask { properties.set("SMask", mask); }
    document.add_object(Object::Stream(Stream::new(properties, image.rgb.clone())))
}

struct ScenePreflight {
    pages_root: ObjectId,
    source_pages: Vec<ObjectId>,
}

fn page_has_widget(document: &Document, page: ObjectId) -> Result<bool, String> {
    let Some(annotations) = document.get_dictionary(page).map_err(err)?.get(b"Annots").ok() else { return Ok(false); };
    let annotations = resolve(document, annotations)?.as_array().map_err(err)?;
    annotations.iter().map(|annotation| {
        let id = annotation.as_reference().map_err(err)?;
        let dictionary = document.get_dictionary(id).map_err(err)?;
        Ok(dictionary.get(b"Subtype").ok().and_then(|value| value.as_name().ok()) == Some(b"Widget"))
    }).collect::<Result<Vec<_>, String>>().map(|values| values.into_iter().any(|value| value))
}

fn scene_preflight(document: &Document, scene: &PdfScene, source_pages: &[ObjectId], identity: bool, pure_reorder: bool) -> Result<ScenePreflight, String> {
    let common = super::signed_mutation_preflight(document, PdfMutationIntent::ValidateOnly)?;
    if common.pages != source_pages { return Err("PDF uses a nested or unsupported page tree; scene export was rejected before output".into()); }
    let pages_root = common.pages_root;

    let mut seen = HashSet::new();
    let mut retained = HashSet::new();
    let mut navigation_geometry_edit = false;
    for page in &scene.pages {
        if let Some(index) = page.source_index {
            let source = *source_pages.get(index).ok_or("Scene source page is outside the document")?;
            if !seen.insert(source) { return Err("A source page can appear only once in a scene".into()); }
            retained.insert(source);
        }
    }
    let has_navigation = common.has_navigation;
    let has_tagged_structure = has_tagged_structure(document, source_pages, common.has_tagged_structure)?;
    if has_tagged_structure && !identity && !pure_reorder {
        return Err("This tagged PDF contains structure mappings that cannot be remapped by scene edits; scene export was rejected before output".into());
    }
    if retained.len() != source_pages.len() && (has_navigation || common.has_form_structure) {
        return Err("This PDF contains navigation or form structures that could target a removed page; scene export was rejected before output".into());
    }
    if retained.len() != source_pages.len() && has_tagged_structure {
        return Err("This tagged PDF contains a StructTreeRoot or ParentTree that could target a removed page; scene export was rejected before output".into());
    }
    for page in &scene.pages {
        if let Some(index) = page.source_index {
            let page_id = source_pages[index];
            let page_dict = document.get_dictionary(page_id).map_err(err)?;
            let has_annotations = page_dict.get(b"Annots").is_ok();
            let has_widget = page_has_widget(document, page_id)?;
            let source_rotation = inherited(document, page_id, b"Rotate")?.map(|value| value.as_i64().map_err(err)).transpose()?.unwrap_or(0).rem_euclid(360) as i32;
            let geometry = geometry(document, page_id)?;
            if let Some(baseline_rotation) = page.source_rotation {
                if baseline_rotation.rem_euclid(360) != source_rotation {
                    return Err("Scene source rotation baseline does not match the source PDF".into());
                }
            }
            if let Some(baseline_box) = &page.source_box {
                let baseline = [baseline_box.x, baseline_box.y, baseline_box.x + baseline_box.width, baseline_box.y + baseline_box.height];
                if !same_box(geometry.bbox, baseline) { return Err("Scene source page-box baseline does not match the source PDF".into()); }
            }
            let changes_page_coordinates = page_geometry_changed(page, &geometry);
            navigation_geometry_edit |= changes_page_coordinates;
            if has_annotations && changes_page_coordinates {
                return Err("This PDF page has annotations or form widgets and the requested crop or rotation would change their coordinates; scene export was rejected before output".into());
            }
            if (page.crop.is_some() || changes_page_coordinates) && has_widget {
                return Err("Cropping or rotating a PDF with form widgets is unsupported; remove the page operation or edit the source PDF first".into());
            }
        }
    }
    if has_navigation && navigation_geometry_edit {
        return Err("This PDF contains bookmarks or destinations whose coordinates cannot be transformed safely after crop or rotation; scene export was rejected before output".into());
    }
    Ok(ScenePreflight { pages_root, source_pages: source_pages.to_vec() })
}

fn same_box(left: [f32; 4], right: [f32; 4]) -> bool { left.iter().zip(right).all(|(left, right)| (*left - right).abs() <= 0.1) }
fn same_rect(left: &Rect, right: &Rect) -> bool { same_box([left.x, left.y, left.x + left.width, left.y + left.height], [right.x, right.y, right.x + right.width, right.y + right.height]) }
fn page_geometry_changed(page: &ScenePage, geometry: &Geometry) -> bool {
    let full_page = Rect { x: 0., y: 0., width: page.width, height: page.height };
    page.crop.as_ref().is_some_and(|crop| !same_rect(crop, &full_page))
        || (page.width - geometry.width).abs() > 0.1
        || (page.height - geometry.height).abs() > 0.1
        || page.rotation.rem_euclid(360) != 0
}
fn inverse_matrix([a, b, c, d, e, f]: [f32; 6]) -> [f32; 6] {
    let determinant = a * d - b * c;
    [d / determinant, -b / determinant, -c / determinant, a / determinant,
        (c * f - d * e) / determinant, (b * e - a * f) / determinant]
        .map(|value| if value == 0. { 0. } else { value })
}
fn layer_contents(behind: Option<Object>, original: Option<Object>, front: Option<Object>) -> Object {
    let original = match original { Some(Object::Array(contents)) => contents, Some(contents) => vec![contents], None => Vec::new() };
    let contents: Vec<Object> = behind.into_iter().chain(original).chain(front).collect();
    if contents.len() == 1 { contents.into_iter().next().unwrap() } else { Object::Array(contents) }
}

fn value_has_tagged_marker(value: &Object) -> bool {
    let dictionary = match value {
        Object::Dictionary(dictionary) => dictionary,
        Object::Stream(stream) => &stream.dict,
        _ => return false,
    };
    dictionary.has(b"ParentTree") || dictionary.has(b"StructParents")
        || dictionary.get(b"Type").ok().and_then(|value| value.as_name().ok()) == Some(b"StructTreeRoot")
}
fn content_has_mcid(value: &Object) -> bool {
    match value {
        Object::Stream(stream) => stream.content.windows(5).any(|window| window == b"/MCID"),
        _ => false,
    }
}
fn page_has_tagged_content(document: &Document, page: ObjectId) -> Result<bool, String> {
    let Some(contents) = document.get_dictionary(page).map_err(err)?.get(b"Contents").ok() else { return Ok(false); };
    match resolve(document, contents)? {
        Object::Array(contents) => contents.iter().map(|content| Ok(content_has_mcid(resolve(document, content)?))).collect::<Result<Vec<_>, String>>().map(|values| values.into_iter().any(|value| value)),
        value => Ok(content_has_mcid(value)),
    }
}
fn has_tagged_structure(document: &Document, pages: &[ObjectId], common: bool) -> Result<bool, String> {
    if common || document.objects.values().any(value_has_tagged_marker) { return Ok(true); }
    pages.iter().map(|page| {
        let dictionary = document.get_dictionary(*page).map_err(err)?;
        Ok(dictionary.has(b"StructParents") || page_has_tagged_content(document, *page)? )
    }).collect::<Result<Vec<_>, String>>().map(|values| values.into_iter().any(|value| value))
}

fn is_identity_scene(document: &Document, scene: &PdfScene, source_pages: &[ObjectId]) -> Result<bool, String> {
    if scene.pages.len() != source_pages.len() { return Ok(false); }
    for (index, page) in scene.pages.iter().enumerate() {
        if page.source_index != Some(index) || !page.objects.is_empty() || page.rotation.rem_euclid(360) != 0 { return Ok(false); }
        dimensions(page.width, page.height)?;
        let source = source_pages[index];
        let geometry = geometry(document, source)?;
        if (page.width - geometry.width).abs() > 0.1 || (page.height - geometry.height).abs() > 0.1 { return Ok(false); }
        let full_page = Rect { x: 0., y: 0., width: page.width, height: page.height };
        if let Some(crop) = &page.crop {
            valid_rect(crop)?;
            if crop.x < 0. || crop.y < 0. || crop.x + crop.width > page.width + 0.01 || crop.y + crop.height > page.height + 0.01 { return Err("Crop is outside the page".into()); }
            if !same_rect(crop, &full_page) { return Ok(false); }
        }
    }
    Ok(true)
}

fn is_pure_page_reorder(document: &Document, scene: &PdfScene, source_pages: &[ObjectId]) -> Result<bool, String> {
    if scene.pages.len() != source_pages.len() {
        return Ok(false);
    }
    let mut seen = HashSet::new();
    for page in &scene.pages {
        let Some(index) = page.source_index else { return Ok(false); };
        let Some(source) = source_pages.get(index).copied() else { return Ok(false); };
        if !seen.insert(index) || !page.objects.is_empty() {
            return Ok(false);
        }
        dimensions(page.width, page.height)?;
        if page_geometry_changed(page, &geometry(document, source)?) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn effective_resource_entries(document: &Document, page_id: ObjectId, category: &[u8]) -> Result<lopdf::Dictionary, String> {
    let Some(value) = inherited(document, page_id, b"Resources")? else { return Ok(lopdf::Dictionary::new()); };
    let resources = value.as_dict().map_err(err)?;
    let Some(entries) = resources.get(category).ok() else { return Ok(lopdf::Dictionary::new()); };
    Ok(resolve(document, entries)?.as_dict().map_err(err)?.clone())
}

fn next_resource_name(entries: &lopdf::Dictionary, category: &str, base: &str) -> Result<String, String> {
    if !entries.has(base.as_bytes()) { return Ok(base.to_string()); }
    for suffix in 1..=10000 {
        let candidate = format!("{base}{suffix}");
        if !entries.has(candidate.as_bytes()) { return Ok(candidate); }
    }
    Err(format!("Could not allocate a unique {category} resource name"))
}

fn scene_resources(document: &Document, page_id: ObjectId, xobjects: lopdf::Dictionary, fonts: lopdf::Dictionary, states: lopdf::Dictionary) -> Result<lopdf::Dictionary, String> {
    let mut resources = match inherited(document, page_id, b"Resources")? {
        Some(value) => value.as_dict().map_err(err)?.clone(),
        None => lopdf::Dictionary::new(),
    };
    let mut page_xobjects = resources.get(b"XObject").ok().map(|value| resolve(document, value).and_then(|value| value.as_dict().map_err(err).map(Clone::clone))).transpose()?.unwrap_or_default();
    for (name, value) in xobjects.into_iter() { page_xobjects.set(name, value); }
    resources.set("XObject", page_xobjects);
    let mut page_fonts = resources.get(b"Font").ok().map(|value| resolve(document, value).and_then(|value| value.as_dict().map_err(err).map(Clone::clone))).transpose()?.unwrap_or_default();
    for (name, value) in fonts.into_iter() { page_fonts.set(name, value); }
    resources.set("Font", page_fonts);
    let mut page_states = resources.get(b"ExtGState").ok().map(|value| resolve(document, value).and_then(|value| value.as_dict().map_err(err).map(Clone::clone))).transpose()?.unwrap_or_default();
    for (name, value) in states.into_iter() { page_states.set(name, value); }
    resources.set("ExtGState", page_states);
    Ok(resources)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage { FillForms, EditText, Overlays, Flatten, Redact }
pub const STAGES: [Stage; 5] = [Stage::FillForms, Stage::EditText, Stage::Overlays, Stage::Flatten, Stage::Redact];
/// Suffix for an export that more than one stage changed.
pub const COMBINED_SUFFIX: &str = "-edited";

impl Stage {
    pub fn suffix(self) -> &'static str {
        match self {
            Stage::FillForms => "-filled",
            Stage::EditText | Stage::Overlays => "-edited",
            Stage::Flatten => "-flattened",
            Stage::Redact => "-redacted",
        }
    }
}

/// The output suffix for the stages that changed the document, or None when nothing did.
pub fn export_suffix(contributors: &[Stage]) -> Option<&'static str> {
    match contributors {
        [] => None,
        [stage] => Some(stage.suffix()),
        _ => Some(COMBINED_SUFFIX),
    }
}

struct OverlayPlan { preflight: ScenePreflight, identity: bool, pure_reorder: bool }
struct RedactPlan { renderer: PathBuf, pages: BTreeMap<usize, Vec<(Rect, [u8; 3])>> }
enum StagePlan { Skip, FillForms(forms::FormPlan), EditText(text_edit::TextEditPlan), Overlays(OverlayPlan), Redact(RedactPlan) }

impl StagePlan {
    fn changes_document(&self) -> bool {
        match self {
            StagePlan::Skip => false,
            StagePlan::FillForms(plan) => plan.changes_document(),
            StagePlan::EditText(_) => true,
            StagePlan::Overlays(plan) => !plan.identity,
            StagePlan::Redact(_) => true,
        }
    }
}

fn unsupported_section(present: bool, name: &str) -> Result<StagePlan, ToolError> {
    if present { return Err(ToolError::processing(format!("{name} is not supported yet; scene export was rejected before output"))); }
    Ok(StagePlan::Skip)
}

fn edit_text_preflight(doc: &Document, edits: Option<&[text_edit::TextEdit]>) -> Result<StagePlan, ToolError> {
    let Some(edits) = edits else { return Ok(StagePlan::Skip) };
    text_edit::preflight(doc, edits, |page| geometry(doc, page).map(|geometry| geometry.matrix)).map(StagePlan::EditText)
}

fn overlay_preflight(doc: &Document, scene: &PdfScene) -> Result<StagePlan, String> {
    if scene.pages.is_empty() || scene.pages.len()>2000 { return Err("Scene must contain 1–2000 pages".into()); }
    let sources:Vec<_>=doc.get_pages().values().copied().collect();
    let identity = is_identity_scene(doc, scene, &sources)?;
    let pure_reorder = is_pure_page_reorder(doc, scene, &sources)?;
    let preflight = scene_preflight(doc, scene, &sources, identity, pure_reorder)?;
    Ok(StagePlan::Overlays(OverlayPlan { preflight, identity, pure_reorder }))
}

fn stage_preflight(stage: Stage, doc: &Document, scene: &PdfScene, sections: &StageSections) -> Result<StagePlan, ToolError> {
    match stage {
        Stage::FillForms => match &sections.form_values {
            Some(values) => forms::preflight(doc, values).map(StagePlan::FillForms),
            None => Ok(StagePlan::Skip),
        },
        Stage::EditText => edit_text_preflight(doc, sections.text_edits.as_deref()),
        Stage::Overlays => overlay_preflight(doc, scene).map_err(ToolError::processing),
        Stage::Flatten => unsupported_section(sections.flatten.is_some(), "Flattening"),
        Stage::Redact => sections.redactions.as_deref().map_or(Ok(StagePlan::Skip), |marks| redact_preflight(scene, marks).map_err(ToolError::processing)),
    }
}


fn stage_apply(plan: StagePlan, doc: &mut Document, scene: &PdfScene) -> Result<(), String> {
    match plan {
        StagePlan::Skip => Ok(()),
        StagePlan::FillForms(plan) => forms::apply(doc, plan),
        StagePlan::EditText(plan) => text_edit::apply(doc, plan),
        StagePlan::Overlays(plan) => apply_overlays(doc, scene, plan),
        StagePlan::Redact(plan) => apply_redactions(doc, scene, plan),
    }
}

/// Runs every stage in order and reports which stages actually changed the document.
fn run_stages(path:&Path, scene:&PdfScene, sections:&StageSections)->Result<(Document, Vec<Stage>),ToolError> { run_stages_counted(path,scene,sections).map(|(doc,contributors,_,_)|(doc,contributors)) }

/// Runs the stages and also returns how many pages the source had, so the export detail can count removed pages,
/// and whether the source was signed. A signed source runs the stages like any other, then loses its signature,
/// because the copy can't keep a valid one; the export reports that with the shared warning.
fn run_stages_counted(path:&Path, scene:&PdfScene, sections:&StageSections)->Result<(Document, Vec<Stage>, usize, bool),ToolError> {
    let mut doc=load(path).map_err(|e| if e==LOCKED_PDF_MESSAGE { locked_pdf() } else { ToolError::processing(e) })?;
    let source_pages=doc.get_pages().len();
    let signed=super::has_signed_signature_field(&doc);
    let plans=STAGES.iter().map(|&stage| stage_preflight(stage, &doc, scene, sections)).collect::<Result<Vec<_>,_>>()?;
    let contributors=STAGES.iter().zip(&plans).filter(|(_, plan)| plan.changes_document()).map(|(&stage, _)| stage).collect();
    for plan in plans { stage_apply(plan, &mut doc, scene).map_err(ToolError::processing)?; }
    if signed { super::remove_signatures(&mut doc).map_err(ToolError::processing)?; }
    Ok((doc, contributors, source_pages, signed))
}
fn compose_with(path:&Path, scene:&PdfScene, sections:&StageSections)->Result<Document,String> { run_stages(path, scene, sections).map(|(doc, _)| doc).map_err(|error| error.message) }

pub fn compose(path:&Path, scene:&PdfScene)->Result<Document,String> { compose_with(path, scene, &StageSections::default()) }

fn apply_overlays(doc:&mut Document, scene:&PdfScene, OverlayPlan { preflight, identity, pure_reorder }: OverlayPlan)->Result<(),String> {
    let sources=&preflight.source_pages;
    if identity { return Ok(()); }
    if pure_reorder {
        let order = scene.pages.iter().map(|page| {
            let index = page.source_index.ok_or_else(|| "Pure page reorder is missing a source page".to_string())?;
            sources.get(index).copied().ok_or_else(|| "Scene source page is outside the document".to_string())
        }).collect::<Result<Vec<_>, String>>()?;
        let root = doc.get_dictionary_mut(preflight.pages_root).map_err(err)?;
        root.set("Kids", order.iter().map(|page| Object::Reference(*page)).collect::<Vec<_>>());
        root.set("Count", order.len() as i64);
        return Ok(());
    }
    let font_specs: [(&str, &str); 9] = [
        ("Helvetica", "SceneFont"), ("Helvetica-Bold", "SceneFontHelveticaBold"),
        ("Helvetica-Oblique", "SceneFontHelveticaOblique"), ("Times-Roman", "SceneFontTimesRoman"),
        ("Times-Bold", "SceneFontTimesBold"), ("Times-Italic", "SceneFontTimesItalic"),
        ("Courier", "SceneFontCourier"), ("Courier-Bold", "SceneFontCourierBold"),
        ("Courier-Oblique", "SceneFontCourierOblique"),
    ];
    let fonts: HashMap<&'static str, ObjectId> = font_specs.iter().map(|(base, resource)| (*resource, doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => *base, "Encoding" => "WinAnsiEncoding"
    }))).collect();
    let text = text_plan(scene).map_err(|error| error.message)?;
    let text_fonts = embed_text_fonts(doc, &text.layouts)?;
    // Each watermark picture is embedded once and shared by every page in its range.
    let mut pictures = HashMap::new();
    for (origin, page) in scene.pages.iter().enumerate() {
        for (index, o) in page.objects.iter().enumerate() {
            if let Some(path) = o.watermark.as_ref().and_then(|options| options.logo_path.as_ref()) {
                let image = watermark_image(path)?;
                let size = (image.width, image.height);
                pictures.insert((origin, index), (add_signature_image(doc, &image), size));
            }
        }
    }
    let mut signature_ids = HashMap::new();
    let mut kids=Vec::new();
    let total=scene.pages.len();
    for (page_index, page) in scene.pages.iter().enumerate() {
        report_progress(page_index, total);
        let placed = &text.placed[page_index];
        dimensions(page.width,page.height)?;
        if page.rotation%90!=0 { return Err("Rotation must be a multiple of 90".into()); }
        let crop=page.crop.clone().unwrap_or(Rect{x:0.,y:0.,width:page.width,height:page.height});
        valid_rect(&crop)?;
        if crop.x<0. || crop.y<0. || crop.x+crop.width>page.width+0.01 || crop.y+crop.height>page.height+0.01 { return Err("Crop is outside the page".into()); }
        let source_index = page.source_index;
        let source_geometry = source_index.map(|index| {
            let id = *sources.get(index).ok_or("Source page index outside document")?;
            geometry(&doc, id)
        }).transpose()?;
        let preserve_source_page = source_geometry.as_ref().is_some_and(|geometry| !page_geometry_changed(page, geometry));
        if preserve_source_page && placed.is_empty() {
            kids.push(Object::Reference(sources[page.source_index.unwrap()]));
            continue;
        }
        let mut effective_fonts = source_index.map(|index| effective_resource_entries(&doc, sources[index], b"Font")).transpose()?.unwrap_or_default();
        let mut font_names = HashMap::new();
        let mut font_resources = lopdf::Dictionary::new();
        for (_, resource) in &font_specs {
            let name = next_resource_name(&effective_fonts, "Font", resource)?;
            effective_fonts.set(name.as_str(), Object::Null);
            let id = *fonts.get(resource).ok_or("Scene font resource disappeared")?;
            font_resources.set(name.as_str(), Object::Reference(id));
            font_names.insert(*resource, name);
        }
        let mut text_resources = HashMap::new();
        for key in placed { if let Some(layout) = text.layouts.get(key) {
            if text_resources.contains_key(layout.font.name) { continue; }
            let name = next_resource_name(&effective_fonts, "Font", &format!("SceneText{}", layout.font.name.replace('-', "")))?;
            effective_fonts.set(name.as_str(), Object::Null);
            font_resources.set(name.as_str(), Object::Reference(text_fonts[layout.font.name].0));
            text_resources.insert(layout.font.name, name);
        } }
        let mut effective_xobjects = source_index.map(|index| effective_resource_entries(&doc, sources[index], b"XObject")).transpose()?.unwrap_or_default();
        let mut effective_states = source_index.map(|index| effective_resource_entries(&doc, sources[index], b"ExtGState")).transpose()?.unwrap_or_default();
        let mut xobjects=lopdf::Dictionary::new();
        let header=if preserve_source_page {
            let g = source_geometry.as_ref().unwrap();
            format!("q\n{}0 0 {} {} re W n\n", cm(inverse_matrix(g.matrix)), page.width, page.height)
        } else {
            format!("q\n1 0 0 -1 {} {} cm\n0 0 {} {} re W n\n",-crop.x,crop.y+crop.height,page.width,page.height)
        };
        let mut original=String::new();
        if let (Some(index), Some(g)) = (source_index, source_geometry.as_ref()) {
            let id=sources[index];
            if (page.width-g.width).abs()>0.1 || (page.height-g.height).abs()>0.1 { return Err("Scene dimensions do not match source page".into()); }
            if !preserve_source_page {
                let original_name = next_resource_name(&effective_xobjects, "XObject", "Original")?;
                effective_xobjects.set(original_name.as_str(), Object::Null);
                let resources=inherited(&doc,id,b"Resources")?.unwrap_or(Object::Dictionary(dictionary!{}));
                resources.as_dict().map_err(err)?;
                let mut form=dictionary!{"Type"=>"XObject","Subtype"=>"Form","BBox"=>array(&g.bbox),"Resources"=>resources};
                if let Ok(group)=doc.get_dictionary(id).map_err(err)?.get(b"Group") { form.set("Group",group.clone()); }
                let bytes=doc.get_page_content(id);
                let form_id=doc.add_object(Stream::new(form,bytes));
                xobjects.set(original_name.as_str(),form_id);
                original.push_str(&format!("q\n{}\n/{original_name} Do\nQ\n", cm(g.matrix)));
            }
        }
        if placed.len()>10000 { return Err("Too many scene objects".into()); }
        let mut states=lopdf::Dictionary::new();
        let (mut behind, mut front)=(String::new(), String::new());
        let area=crop.clone();
        for (i,key) in placed.iter().enumerate() {
            let o=&scene.pages[key.0].objects[key.1];
            let content = if o.layer == Layer::Behind { &mut behind } else { &mut front };
            valid_rect(&o.rect)?;
            if !o.opacity.is_finite() || !(0. ..=1.).contains(&o.opacity) { return Err("Opacity must be between 0 and 1".into()); }
            if !o.font_size.is_finite() || o.font_size<=0. || o.font_size>1000. { return Err("Invalid font size".into()); }
            if o.highlight_mode.is_some() && !matches!(&o.kind, Kind::Highlight) { return Err("Highlight mode is only valid for highlight objects".into()); }
            let watermark = match o.kind { Kind::Watermark => Some(watermark_options(o).map_err(|error| error.message)?), _ => None };
            let rgb=rgb(watermark.map_or(&o.color, |options| options.color.as_deref().unwrap_or("#000000")))?;
            let opacity = watermark.map_or(o.opacity, |options| options.opacity as f32 / 100.);
            let state_base = format!("S{i}");
            let state_name = next_resource_name(&effective_states, "ExtGState", &state_base)?;
            effective_states.set(state_name.as_str(), Object::Null);
            states.set(state_name.as_str(),dictionary!{"Type"=>"ExtGState","ca"=>opacity,"CA"=>opacity});
            let r=&o.rect;
            content.push_str(&format!("q /{state_name} gs {} {} {} rg {} {} {} RG 2 w 1 J 1 j\n",rgb[0],rgb[1],rgb[2],rgb[0],rgb[1],rgb[2]));
            match o.kind {
                Kind::Highlight=>content.push_str(&format!("{} {} {} {} re f\n",r.x,r.y,r.width,r.height)),
                Kind::Shape=>content.push_str(&shape_content(o.shape.as_ref(), r)),
                Kind::Signature=>{
                    let image = &text.signatures[key];
                    let id = *signature_ids.entry(*key).or_insert_with(|| add_signature_image(doc, image));
                    let name = next_resource_name(&effective_xobjects, "XObject", &format!("Sig{i}"))?;
                    effective_xobjects.set(name.as_str(), Object::Null);
                    xobjects.set(name.as_str(), id);
                    let scale = (r.width / image.width as f32).min(r.height / image.height as f32);
                    let (w, h) = (image.width as f32 * scale, image.height as f32 * scale);
                    let (x, y) = (r.x + (r.width - w) / 2., r.y + (r.height - h) / 2.);
                    content.push_str(&format!("q {w} 0 0 {} {x} {} cm /{name} Do Q\n", -h, y + h));
                },
                Kind::Text=>{
                    let layout = &text.layouts[key];
                    content.push_str(&text_content(o, layout, &text_resources[layout.font.name], &text_fonts[layout.font.name].1));
                },
                Kind::Watermark=>{
                    let options = watermark.ok_or("Watermark options disappeared")?;
                    let layout = text.layouts.get(key);
                    let picture = pictures.get(key).copied();
                    let size = watermark_size(options, &area, layout, picture.map(|(_, size)| size));
                    let mark = match (layout, picture) {
                        (Some(layout), _) => watermark_text_content(options, size, &area, layout, &text_resources[layout.font.name], &text_fonts[layout.font.name].1),
                        (None, Some((id, _))) => {
                            let name = next_resource_name(&effective_xobjects, "XObject", &format!("Wm{i}"))?;
                            effective_xobjects.set(name.as_str(), Object::Null);
                            xobjects.set(name.as_str(), id);
                            format!("{} 0 0 {} {} {} cm /{name} Do\n", size.0, -size.1, -size.0 / 2., size.1 / 2.)
                        }
                        (None, None) => return Err("Add watermark text or choose a picture.".into()),
                    };
                    for center in watermark_centers(&area, size, options.position, options.rotation).map_err(|error| error.message)? {
                        content.push_str(&format!("q {}{mark}Q\n", watermark_rotation(options.rotation, center)));
                    }
                },
            }
            content.push_str("Q\n");
        }
        let resources_set = (xobjects, font_resources, states);
        if let Some(index) = source_index {
            let id = preflight.source_pages[index];
            let (xobjects, font_resources, states) = resources_set;
            let resources = scene_resources(&doc, id, xobjects, font_resources, states)?;
            if preserve_source_page {
                // The untouched source content stays in place, between the behind and front layers.
                let behind = (!behind.is_empty()).then(|| Object::Reference(doc.add_object(Stream::new(dictionary!{},format!("{header}{behind}Q\n").into_bytes()))));
                let front = (!front.is_empty()).then(|| Object::Reference(doc.add_object(Stream::new(dictionary!{},format!("{header}{front}Q\n").into_bytes()))));
                let page_dict = doc.get_dictionary_mut(id).map_err(err)?;
                let original_contents = page_dict.get(b"Contents").ok().cloned();
                page_dict.set("Resources",resources);
                page_dict.set("Contents",layer_contents(behind,original_contents,front));
            } else {
                let stream=doc.add_object(Stream::new(dictionary!{},format!("{header}{behind}{original}{front}Q\n").into_bytes()));
                super::set_page_box_family(doc, id, [0., 0., crop.width, crop.height], false)?;
                let page_dict = doc.get_dictionary_mut(id).map_err(err)?;
                page_dict.set("Rotate",page.rotation.rem_euclid(360));
                page_dict.set("Resources",resources);
                page_dict.set("Contents",stream);
            }
            kids.push(Object::Reference(id));
        } else {
            let (xobjects, font_resources, states) = resources_set;
            let stream=doc.add_object(Stream::new(dictionary!{},format!("{header}{behind}{original}{front}Q\n").into_bytes()));
            let id=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>preflight.pages_root,"MediaBox"=>array(&[0.,0.,crop.width,crop.height]),"Rotate"=>page.rotation.rem_euclid(360),"Resources"=>dictionary!{"XObject"=>xobjects,"Font"=>font_resources,"ExtGState"=>states},"Contents"=>stream});
            kids.push(Object::Reference(id));
        }
    }
    doc.objects.get_mut(&preflight.pages_root).ok_or("PDF page-tree root disappeared during scene export")?.as_dict_mut().map_err(err)?.set("Kids",kids);
    doc.objects.get_mut(&preflight.pages_root).ok_or("PDF page-tree root disappeared during scene export")?.as_dict_mut().map_err(err)?.set("Count",scene.pages.len() as i64);
    Ok(())
}

const REDACTION_DPI: f32 = 300.;
const REDACTION_MAX_PIXELS: f32 = 150_000_000.;
const REDACTION_MAX_MARKS: usize = 10_000;
const REDACTION_RENDER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

fn visible(page:&ScenePage)->Rect { page.crop.clone().unwrap_or(Rect{x:0.,y:0.,width:page.width,height:page.height}) }

/// Maps a scene rectangle onto the page as displayed, mirroring the canvas's `sceneTransform`.
/// Returns the mapped rectangle and the displayed page size in points.
fn displayed(page:&ScenePage, r:&Rect)->(Rect,f32,f32) {
    let b=visible(page);
    let (c,s)=match page.rotation.rem_euclid(360) { 90=>(0.,1.), 180=>(-1.,0.), 270=>(0.,-1.), _=>(1.,0.) };
    let map=|x:f32,y:f32| (c*(x-b.x)-s*(y-b.y), s*(x-b.x)+c*(y-b.y));
    let frame=[map(b.x,b.y),map(b.x+b.width,b.y),map(b.x,b.y+b.height),map(b.x+b.width,b.y+b.height)];
    let (min_x,min_y)=frame.iter().fold((f32::MAX,f32::MAX),|(x,y),p|(x.min(p.0),y.min(p.1)));
    let (max_x,max_y)=frame.iter().fold((f32::MIN,f32::MIN),|(x,y),p|(x.max(p.0),y.max(p.1)));
    let corners=[map(r.x,r.y),map(r.x+r.width,r.y+r.height)];
    let (x0,x1)=(corners[0].0.min(corners[1].0)-min_x, corners[0].0.max(corners[1].0)-min_x);
    let (y0,y1)=(corners[0].1.min(corners[1].1)-min_y, corners[0].1.max(corners[1].1)-min_y);
    (Rect{x:x0,y:y0,width:x1-x0,height:y1-y0}, max_x-min_x, max_y-min_y)
}

fn page_list(pages:&[usize])->String {
    let names:Vec<String>=pages.iter().map(|page|(page+1).to_string()).collect();
    match names.as_slice() {
        [one]=>format!("page {one}"),
        [rest@..,last]=>format!("pages {} and {last}",rest.join(", ")),
        []=>String::new(),
    }
}

fn redact_preflight(scene:&PdfScene, marks:&[RedactionMark])->Result<StagePlan,String> {
    if marks.is_empty() || marks.len()>REDACTION_MAX_MARKS { return Err(format!("Redaction needs between 1 and {REDACTION_MAX_MARKS} marks; scene export was rejected before output")); }
    let mut pages:BTreeMap<usize,Vec<(Rect,[u8;3])>>=BTreeMap::new();
    for mark in marks {
        let page=scene.pages.get(mark.page_index).ok_or("A redaction mark is on a page that is not in the document")?;
        valid_rect(&mark.rect)?;
        let b=visible(page);
        let (x0,y0)=(mark.rect.x.max(b.x),mark.rect.y.max(b.y));
        let (x1,y1)=((mark.rect.x+mark.rect.width).min(b.x+b.width),(mark.rect.y+mark.rect.height).min(b.y+b.height));
        if x1<=x0 || y1<=y0 { return Err(format!("A redaction mark is outside page {}; scene export was rejected before output",mark.page_index+1)); }
        let color=rgb(&mark.color)?.map(|value|(value*255.).round() as u8);
        pages.entry(mark.page_index).or_default().push((Rect{x:x0,y:y0,width:x1-x0,height:y1-y0},color));
    }
    let scale=REDACTION_DPI/72.;
    for &index in pages.keys() {
        let b=visible(&scene.pages[index]);
        if b.width*scale*b.height*scale>REDACTION_MAX_PIXELS { return Err(format!("Page {} is too large to redact at 300 dpi; scene export was rejected before output",index+1)); }
    }
    let renderer=renderer().map_err(|_|"Redaction needs the PDF renderer (pdftoppm), which was not found; scene export was rejected before output".to_string())?;
    Ok(StagePlan::Redact(RedactPlan{renderer,pages}))
}

/// Replaces each marked page with a 300 dpi image of itself with the marks painted in, so nothing
/// under a mark survives, then removes hidden data from the whole document.
fn apply_redactions(doc:&mut Document, scene:&PdfScene, RedactPlan{renderer,pages:marked}:RedactPlan)->Result<(),String> {
    let temp=TempDir::new()?;
    let source=temp.0.join("redact.pdf");
    doc.save(&source).map_err(err)?;
    let page_ids:Vec<ObjectId>=doc.get_pages().values().copied().collect();
    if page_ids.len()!=scene.pages.len() { return Err("The composed PDF does not match the scene; redaction stopped before output".into()); }
    let mut removed_annotations=HashSet::new();
    for (&index,marks) in &marked {
        let page=&scene.pages[index];
        let (_,width,height)=displayed(page,&visible(page));
        let prefix=temp.0.join(format!("page-{}",index+1));
        let output=super::run_bounded_helper({
            let mut command=Command::new(&renderer);
            command.args(["-r","300","-cropbox","-png","-singlefile","-f"]).arg((index+1).to_string()).arg("-l").arg((index+1).to_string()).arg(&source).arg(&prefix);
            command
        }, Instant::now()+REDACTION_RENDER_TIMEOUT, 64*1024, super::PDF_TEXT_MAX_ERROR_BYTES, "Rendering a page for redaction timed out.").map_err(err)?;
        if !output.status.success() { return Err(format!("Page {} could not be rendered for redaction",index+1)); }
        let mut image=image::open(prefix.with_extension("png")).map_err(|e|format!("Could not read the rendered page {}: {e}",index+1))?.into_rgb8();
        let (pixel_width,pixel_height)=image.dimensions();
        let (sx,sy)=(pixel_width as f32/width,pixel_height as f32/height);
        for (rect,color) in marks {
            let (r,_,_)=displayed(page,rect);
            let span=|start:f32,end:f32,scale:f32,limit:u32|(((start*scale).floor().max(0.) as u32).min(limit),((end*scale).ceil().max(0.) as u32).min(limit));
            let (x0,x1)=span(r.x,r.x+r.width,sx,pixel_width);
            let (y0,y1)=span(r.y,r.y+r.height,sy,pixel_height);
            for y in y0..y1 { for x in x0..x1 { image.put_pixel(x,y,image::Rgb(*color)); } }
        }
        let image_id=doc.add_object(Stream::new(dictionary!{
            "Type"=>"XObject","Subtype"=>"Image","Width"=>pixel_width as i64,"Height"=>pixel_height as i64,
            "ColorSpace"=>"DeviceRGB","BitsPerComponent"=>8,"Filter"=>"FlateDecode",
        },compress_bytes(image.as_raw())?));
        let content_id=doc.add_object(Stream::new(dictionary!{},format!("q {width} 0 0 {height} 0 0 cm /Redacted Do Q").into_bytes()));
        let original=doc.get_dictionary(page_ids[index]).map_err(err)?;
        let parent=original.get(b"Parent").map_err(err)?.clone();
        if let Ok(annotations)=original.get(b"Annots") {
            removed_annotations.extend(resolve(doc,annotations)?.as_array().map_err(err)?.iter().filter_map(|annotation|annotation.as_reference().ok()));
        }
        let page_box=array(&[0.,0.,width,height]);
        *doc.get_dictionary_mut(page_ids[index]).map_err(err)?=dictionary!{
            "Type"=>"Page","Parent"=>parent,"MediaBox"=>page_box.clone(),"CropBox"=>page_box,"Rotate"=>0,
            "Resources"=>dictionary!{"XObject"=>dictionary!{"Redacted"=>image_id}},"Contents"=>content_id,
        };
    }
    remove_hidden_data(doc,&page_ids,&marked,&removed_annotations)?;
    doc.prune_objects();
    Ok(())
}

fn is_file_attachment(doc:&Document, annotation:&Object)->bool {
    resolve(doc,annotation).ok().and_then(|value|value.as_dict().ok()).and_then(|dict|dict.get(b"Subtype").ok()).and_then(|value|value.as_name().ok())==Some(b"FileAttachment")
}

fn edit_catalog_entry(doc:&mut Document, catalog:ObjectId, key:&[u8], edit:impl FnOnce(&Document,&mut lopdf::Dictionary)->Result<(),String>)->Result<(),String> {
    let Ok(value)=doc.get_dictionary(catalog).map_err(err)?.get(key).cloned() else { return Ok(()); };
    let mut dict=resolve(doc,&value)?.as_dict().map_err(err)?.clone();
    edit(doc,&mut dict)?;
    let catalog=doc.get_dictionary_mut(catalog).map_err(err)?;
    if dict.is_empty() { catalog.remove(key); } else { catalog.set(key.to_vec(),Object::Dictionary(dict)); }
    Ok(())
}

/// Drops form fields whose widgets were removed, returning the retained list and kid-list rewrites.
fn retained_fields(doc:&Document, fields:&[Object], removed:&HashSet<ObjectId>, depth:usize, rewrites:&mut Vec<(ObjectId,Vec<Object>)>)->Result<Vec<Object>,String> {
    if depth>64 { return Err("PDF form fields are nested too deeply to redact safely".into()); }
    let mut kept=Vec::new();
    for field in fields {
        let Ok(id)=field.as_reference() else { kept.push(field.clone()); continue; };
        if removed.contains(&id) { continue; }
        if let Ok(kids)=doc.get_dictionary(id).map_err(err)?.get(b"Kids") {
            let kids=resolve(doc,kids)?.as_array().map_err(err)?.clone();
            let retained=retained_fields(doc,&kids,removed,depth+1,rewrites)?;
            if retained.is_empty() && !kids.is_empty() { continue; }
            if retained.len()!=kids.len() { rewrites.push((id,retained)); }
        }
        kept.push(field.clone());
    }
    Ok(kept)
}

/// Metadata, XMP, attachments, structure text and orphaned form fields could still reveal what was
/// redacted, so they are removed from the whole copy.
fn remove_hidden_data(doc:&mut Document, page_ids:&[ObjectId], redacted:&BTreeMap<usize,Vec<(Rect,[u8;3])>>, removed:&HashSet<ObjectId>)->Result<(),String> {
    doc.trailer.remove(b"Info");
    for (index,&id) in page_ids.iter().enumerate() {
        if redacted.contains_key(&index) { continue; }
        let page=doc.get_dictionary(id).map_err(err)?;
        let annotations=match page.get(b"Annots") { Ok(value)=>resolve(doc,value)?.as_array().map_err(err)?.clone(), Err(_)=>Vec::new() };
        let kept:Vec<Object>=annotations.iter().filter(|annotation|!is_file_attachment(doc,annotation)).cloned().collect();
        let has_files=page.has(b"AF");
        if kept.len()==annotations.len() && !has_files { continue; }
        let page=doc.get_dictionary_mut(id).map_err(err)?;
        page.remove(b"AF");
        if kept.is_empty() { page.remove(b"Annots"); } else if kept.len()!=annotations.len() { page.set("Annots",kept); }
    }
    let catalog=doc.trailer.get(b"Root").and_then(Object::as_reference).map_err(err)?;
    let entry=doc.get_dictionary_mut(catalog).map_err(err)?;
    for key in [&b"Metadata"[..],b"AF",b"StructTreeRoot",b"MarkInfo",b"PieceInfo"] { entry.remove(key); }
    edit_catalog_entry(doc,catalog,b"Names",|_,names|{ names.remove(b"EmbeddedFiles"); Ok(()) })?;
    let mut rewrites=Vec::new();
    edit_catalog_entry(doc,catalog,b"AcroForm",|doc,form|{
        form.remove(b"XFA");
        let fields=match form.get(b"Fields") { Ok(value)=>resolve(doc,value)?.as_array().map_err(err)?.clone(), Err(_)=>Vec::new() };
        let kept=retained_fields(doc,&fields,removed,0,&mut rewrites)?;
        if kept.is_empty() { *form=lopdf::Dictionary::new(); } else { form.set("Fields",kept); }
        Ok(())
    })?;
    for (id,kids) in rewrites { doc.get_dictionary_mut(id).map_err(err)?.set("Kids",kids); }
    Ok(())
}

static NONCE:AtomicU64=AtomicU64::new(0);
struct TempDir(PathBuf);
impl TempDir {
    fn new()->Result<Self,String> {
        for _ in 0..1000 {
            let path=std::env::temp_dir().join(format!("toolbox-scene-{}-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(err)?.as_nanos(),NONCE.fetch_add(1,Ordering::Relaxed)));
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)] {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&path) { Ok(())=>return Ok(Self(path)),Err(e) if e.kind()==std::io::ErrorKind::AlreadyExists=>continue,Err(e)=>return Err(err(e)) }
        } Err("Could not reserve preview directory".into())
    }
}
impl Drop for TempDir { fn drop(&mut self) { let _=fs::remove_dir_all(&self.0); } }
fn renderer()->Result<PathBuf,String> {
    crate::kit::resources::resolve_helper(&crate::kit::resources::PDF_RENDERER)
}

fn text_extractor()->Option<PathBuf> {
    crate::kit::resources::resolve_helper(&crate::kit::resources::PDF_TEXT_EXTRACTOR).ok()
}

fn xml_attribute(element: &quick_xml::events::BytesStart<'_>, name: &[u8]) -> Option<String> {
    element.attributes().flatten().find(|attribute|attribute.key.as_ref() == name).and_then(|attribute|attribute.normalized_value(XmlVersion::Implicit1_0).ok().map(|value|value.into_owned()))
}

fn xml_number(element: &quick_xml::events::BytesStart<'_>, name: &[u8]) -> Option<f32> { xml_attribute(element,name)?.parse().ok() }

fn extract_text_runs(path: &Path, page_number: usize, width: f32, height: f32, deadline: Instant) -> Option<Vec<PdfTextRun>> {
    let extractor=text_extractor()?;
    let output=super::run_bounded_helper({
        let mut command = Command::new(extractor);
        command.args(["-bbox-layout","-enc","UTF-8","-f"]).arg(page_number.to_string()).arg("-l").arg(page_number.to_string()).arg(path).arg("-");
        command
    }, deadline, super::PDF_TEXT_MAX_OUTPUT_BYTES, super::PDF_TEXT_MAX_ERROR_BYTES, "PDF text extraction timed out.").ok()?;
    if !output.status.success() { return None; }
    let mut reader=XmlReader::from_reader(output.stdout.as_slice());
    reader.config_mut().trim_text(false);
    let mut buffer=Vec::new();
    let mut source_size=(width,height);
    let mut current: Option<(f32,f32,f32,f32,String)>=None;
    let mut runs=Vec::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(element)) if element.local_name().as_ref() == b"page" => {
                source_size=(xml_number(&element,b"width").unwrap_or(width),xml_number(&element,b"height").unwrap_or(height));
                if !source_size.0.is_finite() || !source_size.1.is_finite() || source_size.0 <= 0. || source_size.1 <= 0. { return None; }
            }
            Ok(Event::Start(element)) if element.local_name().as_ref() == b"word" => {
                current=Some((xml_number(&element,b"xMin")?,xml_number(&element,b"yMin")?,xml_number(&element,b"xMax")?,xml_number(&element,b"yMax")?,String::new()));
            }
            Ok(Event::Text(value)) => {
                if let Some((_,_,_,_,text))=current.as_mut() {
                    let decoded=value.decode().ok()?;
                    text.push_str(unescape(&decoded).ok()?.as_ref());
                }
            }
            Ok(Event::End(element)) if element.local_name().as_ref() == b"word" => {
                if let Some((x_min,y_min,x_max,y_max,text))=current.take() {
                    if ![x_min,y_min,x_max,y_max].iter().all(|value| value.is_finite()) { return None; }
                    if !text.trim().is_empty() && runs.len() < 10_000 && x_max > x_min && y_max > y_min {
                        let sx=width/source_size.0; let sy=height/source_size.1;
                        runs.push(PdfTextRun{text,x:x_min*sx,y:y_min*sy,width:(x_max-x_min)*sx,height:(y_max-y_min)*sy});
                    }
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => return None,
        }
        buffer.clear();
    }
    Some(runs)
}
fn render(path:&Path,index:usize,renderer:&Path,temp:&TempDir)->Result<ScenePreview,String> {
    let prefix=temp.0.join("render");
    let output=super::run_bounded_helper({
        let mut command = Command::new(renderer);
        command.args(["-png","-singlefile","-scale-to"]).arg(super::PDF_PREVIEW_MAX_DIMENSION.to_string()).arg("-f").arg((index+1).to_string()).arg("-l").arg((index+1).to_string()).arg(path).arg(&prefix);
        command
    }, Instant::now() + super::PDF_PREVIEW_TIMEOUT, 64 * 1024, super::PDF_TEXT_MAX_ERROR_BYTES, "PDF preview renderer timed out.").map_err(err)?;
    if !output.status.success() { return Err("pdftoppm could not render this PDF".into()); }
    let (bytes,width,height)=super::read_preview_png(&prefix.with_extension("png")).map_err(err)?;
    Ok(ScenePreview{data_url:format!("data:image/png;base64,{}",base64::engine::general_purpose::STANDARD.encode(bytes)),width,height})
}
pub fn preview(request:&PreviewRequest)->Result<ScenePreview,String> {
    if request.page_index>=request.scene.pages.len() { return Err("Preview page outside scene".into()); }
    let renderer=renderer()?;
    let temp=TempDir::new()?;
    let path=temp.0.join("scene.pdf");
    compose_with(&request.path,&request.scene,&request.sections)?.save(&path).map_err(err)?;
    render(&path,request.page_index,&renderer,&temp)
}
fn validate_preview_page_indices(page_indices:&[usize], page_count:usize)->Result<(),String> {
    if !(1..=3).contains(&page_indices.len()) { return Err("Preview requests must include 1–3 page indices".into()); }
    let mut seen=HashSet::with_capacity(page_indices.len());
    for &index in page_indices {
        if index>=page_count { return Err("Preview page outside scene".into()); }
        if !seen.insert(index) { return Err("Preview page indices must be unique".into()); }
    }
    Ok(())
}
pub fn preview_pages(request:&PreviewPagesRequest)->Result<Vec<IndexedScenePreview>,String> {
    validate_preview_page_indices(&request.page_indices,request.scene.pages.len())?;
    let renderer=renderer()?;
    let temp=TempDir::new()?;
    let path=temp.0.join("scene.pdf");
    compose_with(&request.path,&request.scene,&request.sections)?.save(&path).map_err(err)?;
    request.page_indices.iter().copied().map(|page_index| {
        render(&path,page_index,&renderer,&temp).map(|preview| IndexedScenePreview{page_index,preview})
    }).collect()
}
pub const UNREADABLE_PDF_MESSAGE:&str="Toolbox can't read this file. It may be damaged, or it isn't a PDF that Toolbox supports. Nothing was changed.";

/// Reads page geometry for the scene editors. Password-protected and unreadable files come back as typed
/// invalid-input errors so the UI can explain them (and point locked files to File Security).
pub fn inspect(path:&Path)->Result<PdfDocumentMetadata,ToolError> {
    let locked=locked_pdf;
    fn unreadable(_:impl std::fmt::Display)->ToolError { ToolError::invalid_input(UNREADABLE_PDF_MESSAGE) }
    let doc=match Document::load(path) {
        Ok(doc)=>doc,
        Err(lopdf::Error::Decryption(_)|lopdf::Error::InvalidPassword|lopdf::Error::UnsupportedSecurityHandler(_))=>return Err(locked()),
        Err(error)=>return Err(unreadable(error)),
    };
    if doc.is_encrypted() { return Err(locked()); }
    let mut pages=Vec::new();
    for (index,id) in doc.get_pages().values().enumerate() {
        let g=geometry(&doc,*id).map_err(unreadable)?;
        let source_rotation = inherited(&doc, *id, b"Rotate").map_err(unreadable)?.map(|value| value.as_i64().map_err(err)).transpose().map_err(unreadable)?.unwrap_or(0).rem_euclid(360) as i32;
        pages.push(PdfPageMetadata{index,x:g.bbox[0],y:g.bbox[1],width:g.width,height:g.height,rotation:source_rotation,page_box:g.bbox,preview:None,text_runs:None,edit_runs:None});
    }
    if pages.is_empty() { return Err(ToolError::invalid_input("This PDF has no pages. Nothing was changed.")); }
    Ok(PdfDocumentMetadata{path:path.to_path_buf(),pages,signed:super::has_signed_signature_field(&doc)})
}
pub fn inspect_page(path:&Path,page_index:usize)->Result<PdfPageMetadata,String> {
    let doc=load(path)?;
    let pages:Vec<_>=doc.get_pages().values().copied().collect();
    let id=*pages.get(page_index).ok_or("PDF page outside document")?;
    let g=geometry(&doc,id)?;
    let source_rotation = inherited(&doc, id, b"Rotate")?.map(|value| value.as_i64().map_err(err)).transpose()?.unwrap_or(0).rem_euclid(360) as i32;
    let text_runs=extract_text_runs(path,page_index + 1,g.width,g.height,Instant::now() + super::PDF_TEXT_TIMEOUT).unwrap_or_default();
    let preview = super::metadata::render_preview(path, page_index + 1, g.width, g.height);
    let edit_runs=text_edit::page_text(&doc,id,g.matrix).map(|text| text.runs()).unwrap_or_default();
    Ok(PdfPageMetadata{index:page_index,x:g.bbox[0],y:g.bbox[1],width:g.width,height:g.height,rotation:source_rotation,page_box:g.bbox,preview,text_runs:Some(text_runs),edit_runs:Some(edit_runs)})
}
pub const NOTHING_TO_EXPORT: &str = "Nothing to export: this document has no changes, so a copy would match the original.";

/// The detail line for an export, counted against the source so a dropped page reads as removed.
fn page_summary(scene:&PdfScene, source_pages:usize)->String {
    let kept=scene.pages.iter().filter(|page| page.source_index.is_some()).count();
    let referenced=scene.pages.iter().filter_map(|page| page.source_index).collect::<HashSet<_>>().len();
    let removed=source_pages.saturating_sub(referenced);
    let rotated=scene.pages.iter().filter(|page| page.rotation.rem_euclid(360)!=0).count();
    // The scene has no mirror operation yet, so nothing is ever mirrored.
    let mirrored=0;
    let added=scene.pages.len()-kept;
    let pages=if kept==1 { "page" } else { "pages" };
    let added=if added>0 { format!(", {added} blank added") } else { String::new() };
    format!("{kept} {pages} kept, {removed} removed, {rotated} rotated, {mirrored} mirrored{added}.")
}

/// Writes the scene as a new "-edited" copy. The bytes are built in memory first, so a scene that fails leaves nothing behind,
/// and a write that fails part way is dropped with its reservation instead of being published.
pub fn export(request:&ExportRequest,input:PathBuf)->JobOutcome {
    let chosen=request.output_path.as_deref();
    let result=(|| {
        if chosen.is_some() && request.paths.len()!=1 { return Err(ToolError::invalid_input("Export As saves one document at a time")); }
        text_plan(&request.scene)?;
        let (mut doc, contributors, source_pages, signed)=run_stages_counted(&input,&request.scene,&request.sections)?;
        let suffix=export_suffix(&contributors).ok_or_else(|| ToolError::invalid_input(NOTHING_TO_EXPORT))?;
        let mut bytes=Vec::new();
        doc.save_to(&mut bytes).map_err(|e| ToolError::processing(err(e)))?;
        report_progress(request.scene.pages.len(), request.scene.pages.len());
        let (reservation, mut file)=OutputNaming::create_single_output(chosen, &input, &request.output_location, suffix, "pdf")
            .map_err(|e| ToolError::processing(format!("Could not reserve the exported PDF: {e}")))?;
        file.write_all(&bytes).and_then(|_| file.sync_all()).map_err(|e| ToolError::processing(err(e)))?;
        drop(file);
        let path=reservation.publish().map_err(|e| ToolError::processing(err(e)))?;
        Ok::<_,ToolError>((path,page_summary(&request.scene,source_pages),signed))
    })();
    match result {
        Ok((path,summary,signed))=>{
            // Redaction adds what was rasterised and stripped after the page summary every export reports.
            let detail=match &request.sections.redactions {
                Some(marks) if !marks.is_empty()=>{
                    let pages:Vec<usize>=marks.iter().map(|mark|mark.page_index).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
                    let image=if pages.len()==1 {"was redacted as a 300 dpi image"} else {"were redacted as 300 dpi images"};
                    format!("{summary} {} {image}. Document metadata, XMP data and attachments were removed.",page_list(&pages).replacen('p',"P",1))
                }
                _=>summary,
            };
            super::note_signature_removed(note_renamed(JobOutcome::succeeded(input, vec![path], detail), chosen), signed)
        }
        Err(error)=>JobOutcome::failure(input,error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::common::OutputNaming;

    fn fixture(path: &Path) {
        let mut d = Document::with_version("1.7");
        let root = d.new_object_id();
        let font = d.add_object(dictionary! {"Type"=>"Font", "Subtype"=>"Type1", "BaseFont"=>"Helvetica"});
        let fonts = d.add_object(dictionary! {"OriginalFont"=>font});
        let resources = d.add_object(dictionary! {"Font"=>fonts});
        let content = d.add_object(Stream::new(dictionary!{}, b"q 1 0 0 rg 40 60 50 70 re f Q BT /OriginalFont 18 Tf 40 260 Td (Original content) Tj ET".to_vec()));
        let first = d.add_object(dictionary! {"Type"=>"Page", "Parent"=>root, "MediaBox"=>array(&[10.,20.,250.,360.]), "CropBox"=>array(&[20.,30.,220.,330.]), "Rotate"=>90, "Resources"=>resources, "Contents"=>content});
        let second = d.add_object(dictionary! {"Type"=>"Page", "Parent"=>root, "MediaBox"=>array(&[0.,0.,612.,792.]), "Resources"=>resources, "Contents"=>content});
        d.objects.insert(root, Object::Dictionary(dictionary! {"Type"=>"Pages", "Kids"=>vec![Object::Reference(first),Object::Reference(second)], "Count"=>2}));
        let catalog = d.add_object(dictionary! {"Type"=>"Catalog","Pages"=>root});
        d.trailer.set("Root",catalog);
        d.save(path).unwrap();
    }
    fn page_text_fixture(path: &Path) {
        let mut d = Document::with_version("1.7");
        let root = d.new_object_id();
        let font = d.add_object(dictionary! {"Type"=>"Font", "Subtype"=>"Type1", "BaseFont"=>"Helvetica"});
        let fonts = d.add_object(dictionary! {"OriginalFont"=>font});
        let resources = d.add_object(dictionary! {"Font"=>fonts});
        let first_content = d.add_object(Stream::new(dictionary!{}, b"BT /OriginalFont 18 Tf 40 700 Td (First page text) Tj ET".to_vec()));
        let second_content = d.add_object(Stream::new(dictionary!{}, b"BT /OriginalFont 18 Tf 40 700 Td (Second page text) Tj ET".to_vec()));
        let first = d.add_object(dictionary! {"Type"=>"Page", "Parent"=>root, "MediaBox"=>array(&[0.,0.,612.,792.]), "Resources"=>resources, "Contents"=>first_content});
        let second = d.add_object(dictionary! {"Type"=>"Page", "Parent"=>root, "MediaBox"=>array(&[0.,0.,612.,792.]), "Resources"=>resources, "Contents"=>second_content});
        d.objects.insert(root, Object::Dictionary(dictionary! {"Type"=>"Pages", "Kids"=>vec![Object::Reference(first),Object::Reference(second)], "Count"=>2}));
        let catalog = d.add_object(dictionary! {"Type"=>"Catalog","Pages"=>root});
        d.trailer.set("Root",catalog);
        d.save(path).unwrap();
    }
    fn labelled_pages_fixture(path: &Path, labels: &[&str]) {
        let mut d = Document::with_version("1.7");
        let root = d.new_object_id();
        let font = d.add_object(dictionary! {"Type"=>"Font", "Subtype"=>"Type1", "BaseFont"=>"Helvetica"});
        let resources = d.add_object(dictionary! {"Font"=>dictionary! {"OriginalFont"=>font}});
        let kids = labels.iter().map(|label| {
            let content = d.add_object(Stream::new(dictionary!{}, format!("BT /OriginalFont 18 Tf 40 700 Td ({label}) Tj ET").into_bytes()));
            Object::Reference(d.add_object(dictionary! {"Type"=>"Page", "Parent"=>root, "MediaBox"=>array(&[0.,0.,612.,792.]), "Resources"=>resources, "Contents"=>content}))
        }).collect::<Vec<_>>();
        d.objects.insert(root, Object::Dictionary(dictionary! {"Type"=>"Pages", "Kids"=>kids, "Count"=>labels.len() as i64}));
        let catalog = d.add_object(dictionary! {"Type"=>"Catalog","Pages"=>root});
        d.trailer.set("Root",catalog);
        d.save(path).unwrap();
    }
    fn page_labels(path: &Path) -> Vec<String> {
        let d = Document::load(path).unwrap();
        d.get_pages().values().map(|page| String::from_utf8_lossy(&d.get_page_content(*page)).into_owned()).collect()
    }
    fn nested_fixture(path: &Path) {
        let mut d = Document::with_version("1.7");
        let root = d.new_object_id();
        let nested = d.new_object_id();
        let content = d.add_object(Stream::new(dictionary!{}, b"q".to_vec()));
        let first = d.add_object(dictionary! {"Type"=>"Page", "Parent"=>nested, "Contents"=>content});
        let second = d.add_object(dictionary! {"Type"=>"Page", "Parent"=>root, "MediaBox"=>array(&[0.,0.,612.,792.]), "Contents"=>content});
        d.objects.insert(nested, Object::Dictionary(dictionary! {"Type"=>"Pages", "Parent"=>root, "Kids"=>vec![Object::Reference(first)], "Count"=>1,
            "MediaBox"=>array(&[10.,20.,250.,360.]), "CropBox"=>array(&[20.,30.,220.,330.]), "Rotate"=>90}));
        d.objects.insert(root, Object::Dictionary(dictionary! {"Type"=>"Pages", "Kids"=>vec![Object::Reference(nested),Object::Reference(second)], "Count"=>2}));
        let catalog = d.add_object(dictionary! {"Type"=>"Catalog","Pages"=>root});
        d.trailer.set("Root",catalog);
        d.save(path).unwrap();
    }

    fn annotated_rotated_offset_fixture(path: &Path) {
        let mut d = Document::with_version("1.7");
        let pages_id = d.new_object_id();
        let content = d.add_object(Stream::new(dictionary!{}, b"BT /F1 18 Tf 40 260 Td (Annotated source) Tj ET".to_vec()));
        let font = d.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
        let fonts = d.add_object(dictionary! { "F1" => font });
        let resources = d.add_object(dictionary! { "Font" => fonts });
        let annotation = d.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Text", "Rect" => array(&[30., 40., 90., 80.]) });
        let link = d.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Link", "Rect" => array(&[100., 40., 160., 80.]), "A" => dictionary! { "S" => "URI", "URI" => "https://example.invalid" } });
        let widget = d.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx", "Rect" => array(&[170., 40., 210., 80.]) });
        let page = d.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id, "Resources" => resources, "Contents" => content, "Annots" => vec![Object::Reference(annotation), Object::Reference(link), Object::Reference(widget)] });
        d.objects.insert(pages_id, dictionary! {
            "Type" => "Pages", "Kids" => vec![Object::Reference(page)], "Count" => 1,
            "MediaBox" => array(&[10., 20., 250., 360.]), "CropBox" => array(&[20., 30., 220., 330.]),
            "BleedBox" => array(&[22., 32., 218., 328.]), "TrimBox" => array(&[24., 34., 216., 326.]), "ArtBox" => array(&[26., 36., 214., 324.]), "Rotate" => 90,
        }.into());
        let names = d.add_object(dictionary! { "Dests" => dictionary! { "fixture" => Object::Array(vec![Object::Reference(page), Object::Name(b"Fit".to_vec())]) } });
        let outlines = d.add_object(dictionary! { "Type" => "Outlines", "Count" => 0 });
        let acro_form = d.add_object(dictionary! { "Fields" => vec![Object::Reference(widget)] });
        let catalog = d.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id, "Names" => names, "Outlines" => outlines, "AcroForm" => acro_form });
        d.trailer.set("Root", catalog);
        d.save(path).unwrap();
    }

    fn tagged_reorder_fixture(path: &Path) {
        fixture(path);
        let mut d = Document::load(path).unwrap();
        let pages: Vec<_> = d.get_pages().values().copied().collect();
        let link = d.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Link", "Rect" => array(&[20., 20., 80., 50.]),
            "P" => Object::Reference(pages[1]),
            "Dest" => vec![Object::Reference(pages[0]), Object::Name(b"Fit".to_vec())],
        });
        let widget = d.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx", "Rect" => array(&[90., 20., 180., 50.]),
            "P" => Object::Reference(pages[0]),
        });
        d.get_dictionary_mut(pages[0]).unwrap().set("Annots", vec![Object::Reference(widget)]);
        d.get_dictionary_mut(pages[1]).unwrap().set("Annots", vec![Object::Reference(link)]);
        d.get_dictionary_mut(pages[0]).unwrap().set("StructParents", 0);
        d.get_dictionary_mut(pages[1]).unwrap().set("StructParents", 1);

        let metadata = d.add_object(Stream::new(dictionary! { "Type" => "Metadata", "Subtype" => "XML" }, b"<xmpmeta>stable</xmpmeta>".to_vec()));
        let names = d.add_object(dictionary! {
            "Dests" => dictionary! { "first" => vec![Object::Reference(pages[0]), Object::Name(b"Fit".to_vec())] },
        });
        let outlines = d.add_object(dictionary! { "Type" => "Outlines", "Count" => 0 });
        let parent_tree = d.add_object(dictionary! {
            "Nums" => vec![Object::Integer(0), Object::Dictionary(dictionary! {
                "Pg" => Object::Reference(pages[0]),
            })],
        });
        let structure = d.add_object(dictionary! {
            "Type" => "StructTreeRoot", "K" => vec![], "ParentTree" => Object::Reference(parent_tree),
        });
        let acro_form = d.add_object(dictionary! { "Fields" => vec![Object::Reference(widget)] });
        let catalog = d.trailer.get(b"Root").unwrap().as_reference().unwrap();
        let catalog = d.get_dictionary_mut(catalog).unwrap();
        catalog.set("Metadata", Object::Reference(metadata));
        catalog.set("Names", Object::Reference(names));
        catalog.set("Outlines", Object::Reference(outlines));
        catalog.set("StructTreeRoot", Object::Reference(structure));
        catalog.set("AcroForm", Object::Reference(acro_form));
        d.save(path).unwrap();
    }

    fn assert_only_page_tree_order_changed(document: &Document, original: &Document, pages_root: ObjectId, order: &[ObjectId]) {
        assert_eq!(document.trailer, original.trailer);
        assert_eq!(document.objects.len(), original.objects.len());
        for (id, value) in &original.objects {
            if *id == pages_root {
                let mut expected = original.get_dictionary(*id).unwrap().clone();
                expected.set("Kids", order.iter().map(|page| Object::Reference(*page)).collect::<Vec<_>>());
                expected.set("Count", order.len() as i64);
                assert_eq!(document.get_object(*id).unwrap(), &Object::Dictionary(expected));
            } else {
                assert_eq!(document.get_object(*id).unwrap(), value, "object {id:?} changed during pure reorder");
            }
        }
    }

    fn inherited_collision_fixture(path: &Path) {
        let mut d = Document::with_version("1.7");
        let pages_id = d.new_object_id();
        let existing_font = d.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier" });
        let fonts_id = d.add_object(dictionary! { "SceneFont" => existing_font, "SceneFont1" => existing_font, "SceneTextNotoSansRegular" => existing_font });
        let existing_xobject = d.add_object(Stream::new(dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => array(&[0., 0., 1., 1.]) }, Vec::new()));
        let xobjects_id = d.add_object(dictionary! {
            "Original" => existing_xobject, "Original1" => existing_xobject,
            "Sig1" => existing_xobject, "Sig11" => existing_xobject,
        });
        let existing_state = d.add_object(dictionary! { "Type" => "ExtGState", "ca" => 0.1, "CA" => 0.1 });
        let states_id = d.add_object(dictionary! { "S0" => existing_state, "S01" => existing_state, "S1" => existing_state, "S11" => existing_state });
        let resources_id = d.add_object(dictionary! {
            "Font" => Object::Reference(fonts_id), "XObject" => Object::Reference(xobjects_id),
            "ExtGState" => Object::Reference(states_id),
        });
        let content = d.add_object(Stream::new(dictionary! {}, b"BT /SceneFont 18 Tf 40 260 Td (Original content) Tj ET".to_vec()));
        let page = d.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id, "MediaBox" => array(&[0., 0., 612., 792.]), "Contents" => content });
        d.objects.insert(pages_id, dictionary! {
            "Type" => "Pages", "Kids" => vec![Object::Reference(page)], "Count" => 1,
            "Resources" => Object::Reference(resources_id),
        }.into());
        let catalog = d.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        d.trailer.set("Root", catalog);
        d.save(path).unwrap();
    }
    fn object(kind: Kind) -> SceneObject {
        let signature_mode = matches!(kind, Kind::Signature).then_some(SignatureMode::Text);
        let watermark = matches!(kind, Kind::Watermark).then(|| text_watermark("LOCAL"));
        SceneObject { signature_mode, kind, rect: Rect{x:30.,y:40.,width:120.,height:40.}, text:"LOCAL".into(),font_size:18.,color:"#0066CC".into(),opacity:0.5,
            shape:None, highlight_mode:None,
            signature_path:None, font_family:None,
            watermark, layer: Layer::Front,
            bold:false, italic:false, align:TextAlign::Left, page_range:None }
    }
    fn text_watermark(text: &str) -> WatermarkOptions {
        WatermarkOptions { text: Some(text.into()), logo_path: None, size: None, opacity: 30, color: Some("#0000FF".into()), position: None, rotation: 0. }
    }
    fn watermark_object(options: WatermarkOptions, range: Option<&str>) -> SceneObject {
        SceneObject { rect: Rect{x:0.,y:0.,width:612.,height:792.}, watermark: Some(options), page_range: range.map(Into::into), ..object(Kind::Watermark) }
    }
    fn scene() -> PdfScene {
        PdfScene { pages:vec![
            ScenePage {source_index:Some(1),width:612.,height:792.,rotation:180,crop:Some(Rect{x:10.,y:20.,width:500.,height:600.}),source_rotation:None,source_box:None,objects:vec![object(Kind::Highlight),object(Kind::Text),object(Kind::Shape),object(Kind::Watermark),object(Kind::Signature)]},
            ScenePage {source_index:None,width:300.,height:200.,rotation:90,crop:None,source_rotation:None,source_box:None,objects:vec![object(Kind::Text)]},
            ScenePage {source_index:Some(0),width:300.,height:200.,rotation:0,crop:None,source_rotation:None,source_box:None,objects:vec![]},
        ] }
    }

    #[cfg(unix)]
    #[test]
    fn renderer_uses_bundled_pdftoppm_when_path_and_override_are_unavailable() {
        use std::os::unix::fs::PermissionsExt;

        let _guard = crate::kit::lock_process_env();
        let temp = TempDir::new().unwrap();
        let pdf_bin = temp.0.join("pdf-bin");
        fs::create_dir(&pdf_bin).unwrap();
        let bundled = pdf_bin.join("pdftoppm");
        fs::write(&bundled, b"#!/bin/sh\nexit 0\n").unwrap();
        let mut permissions = fs::metadata(&bundled).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&bundled, permissions).unwrap();

        let previous_root = std::env::var_os("TOOLBOX_RESOURCE_ROOT");
        let previous_override = std::env::var_os("TOOLBOX_PDFTOPPM_PATH");
        let previous_path = std::env::var_os("PATH");
        std::env::set_var("TOOLBOX_RESOURCE_ROOT", &temp.0);
        std::env::remove_var("TOOLBOX_PDFTOPPM_PATH");
        std::env::set_var("PATH", temp.0.join("missing-path"));
        let result = renderer();
        match previous_root {
            Some(value) => std::env::set_var("TOOLBOX_RESOURCE_ROOT", value),
            None => std::env::remove_var("TOOLBOX_RESOURCE_ROOT"),
        }
        match previous_override {
            Some(value) => std::env::set_var("TOOLBOX_PDFTOPPM_PATH", value),
            None => std::env::remove_var("TOOLBOX_PDFTOPPM_PATH"),
        }
        match previous_path {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }

        assert_eq!(result.unwrap(), bundled);
    }

    #[test]
    fn geometry_resolves_nested_resources_boxes_and_rotation() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); nested_fixture(&input);
        let d=load(&input).unwrap(); let pages:Vec<_>=d.get_pages().values().copied().collect();
        let g=geometry(&d,pages[0]).unwrap();
        assert_eq!((g.width,g.height),(300.,200.));
        assert_eq!(g.matrix,[0.,1.,1.,0.,-30.,-20.]);
        fixture(&input);
        let composed=compose(&input,&scene()).unwrap();
        let pages:Vec<_>=composed.get_pages().values().copied().collect();
        assert_eq!(pages.len(),3);
        assert_eq!(bounds(&composed,composed.get_dictionary(pages[0]).unwrap().get(b"MediaBox").unwrap()).unwrap(),[0.,0.,500.,600.]);
        assert_eq!(composed.get_dictionary(pages[1]).unwrap().get(b"Rotate").unwrap().as_i64().unwrap(),90);
        let bytes=composed.get_page_content(pages[0]); let content=String::from_utf8_lossy(&bytes);
        assert!(content.contains("/S4 gs")); assert!(content.contains("re W n")); assert!(content.contains("/SceneTextNotoSansRegular 50 Tf"), "a tenth of the cropped shorter side: {content}");
        let blank=composed.get_page_content(pages[1]); assert!(!String::from_utf8_lossy(&blank).contains("/Original Do"));
        assert!(String::from_utf8_lossy(&blank).contains("/SceneTextNotoSansRegular 18 Tf"));
    }

    #[test]
    fn inspect_returns_page_metadata_without_invoking_pdftoppm() {
        let _guard = crate::kit::lock_process_env();
        let temp = TempDir::new().unwrap();
        let input = temp.0.join("source.pdf");
        fixture(&input);
        let previous = std::env::var_os("TOOLBOX_PDFTOPPM_PATH");
        std::env::set_var("TOOLBOX_PDFTOPPM_PATH", temp.0.join("pdftoppm-not-invoked"));
        let result = inspect(&input);
        match previous {
            Some(value) => std::env::set_var("TOOLBOX_PDFTOPPM_PATH", value),
            None => std::env::remove_var("TOOLBOX_PDFTOPPM_PATH"),
        }
        let metadata = result.unwrap();
        assert_eq!(metadata.pages.len(), 2);
        assert_eq!((metadata.pages[0].width, metadata.pages[0].height), (300., 200.));
        assert_eq!(metadata.pages[0].page_box, [20., 30., 220., 330.]);
        assert_eq!(metadata.pages[0].rotation, 90);
        assert!(metadata.pages.iter().all(|page| page.preview.is_none() && page.text_runs.is_none()));
    }

    fn assert_invalid_input_without_output(dir: &Path, input: &Path, message: &str, pointer: Option<&str>) {
        let before = fs::read(input).unwrap();
        let error = inspect(input).expect_err("inspect should reject the fixture");
        assert!(matches!(error.kind, crate::kit::contracts::ErrorKind::InvalidInput), "kind: {:?}", error.kind);
        assert_eq!(error.message, message);
        assert!(error.message.is_ascii() && !error.message.contains("lopdf") && !error.message.contains("Error"));
        match (pointer, &error.pointer) {
            (Some(id), Some(crate::kit::contracts::OutcomePointer::Tool { tool_id })) => assert_eq!(tool_id, id),
            (None, None) => {}
            (expected, actual) => panic!("pointer mismatch: expected {expected:?}, got {actual:?}"),
        }
        assert_eq!(fs::read(input).unwrap(), before, "the original must not change");
        assert_eq!(fs::read_dir(dir).unwrap().count(), 1, "inspect must not write any output");
    }

    #[test]
    fn inspect_rejects_a_password_protected_pdf_with_a_file_security_pointer() {
        let temp = TempDir::new().unwrap();
        let input = temp.0.join("locked.pdf");
        fixture(&input);
        let mut document = Document::load(&input).unwrap();
        document.trailer.set("ID", Object::Array(vec![
            Object::String(b"toolbox-scene-id-1".to_vec(), lopdf::StringFormat::Literal),
            Object::String(b"toolbox-scene-id-2".to_vec(), lopdf::StringFormat::Literal),
        ]));
        let state = lopdf::EncryptionState::try_from(lopdf::EncryptionVersion::V2 {
            document: &document, owner_password: "owner", user_password: "user", key_length: 128, permissions: lopdf::Permissions::all(),
        }).unwrap();
        document.encrypt(&state).unwrap();
        document.save(&input).unwrap();
        assert_invalid_input_without_output(&temp.0, &input, LOCKED_PDF_MESSAGE, Some("lock-unlock"));
    }

    #[test]
    fn inspect_rejects_a_damaged_pdf_with_plain_text() {
        let temp = TempDir::new().unwrap();
        let input = temp.0.join("damaged.pdf");
        fixture(&input);
        let bytes = fs::read(&input).unwrap();
        fs::write(&input, &bytes[..bytes.len() / 3]).unwrap();
        assert_invalid_input_without_output(&temp.0, &input, UNREADABLE_PDF_MESSAGE, None);
        fs::write(&input, b"%PDF-1.7\nthis is not a pdf body").unwrap();
        assert_invalid_input_without_output(&temp.0, &input, UNREADABLE_PDF_MESSAGE, None);
    }

    #[test]
    fn inspect_page_validates_bounds_and_returns_only_the_requested_text_runs() {
        let _guard = crate::kit::lock_process_env();
        if text_extractor().is_none() || renderer().is_err() { return; }
        let temp = TempDir::new().unwrap();
        let input = temp.0.join("page-text.pdf");
        page_text_fixture(&input);

        let page = inspect_page(&input, 1).unwrap();
        assert_eq!(page.index, 1);
        assert_eq!((page.width, page.height), (612., 792.));
        assert!(page.preview.as_deref().is_some_and(|preview| preview.starts_with("data:image/png;base64,")));
        let runs = page.text_runs.expect("page inspection should return a loaded text-run list");
        assert!(runs.iter().any(|run| run.text.contains("Second")), "runs: {runs:?}");
        assert!(runs.iter().all(|run| run.width.is_finite() && run.height.is_finite() && run.width > 0. && run.height > 0.));
        assert!(inspect_page(&input, 2).is_err_and(|error| error == "PDF page outside document"));
    }

    #[test]
    fn inspect_then_noop_export_preserves_annotated_rotated_offset_pdf_geometry() {
        let _guard = crate::kit::lock_process_env();
        if renderer().is_err() { return; }
        let temp = TempDir::new().unwrap();
        let input = temp.0.join("annotated-rotated-offset.pdf");
        annotated_rotated_offset_fixture(&input);
        let metadata = inspect(&input).unwrap();
        let scene = PdfScene { pages: metadata.pages.iter().map(|page| ScenePage {
            source_index: Some(page.index), width: page.width, height: page.height, rotation: 0, crop: None,
            source_rotation: Some(page.rotation),
            source_box: Some(Rect { x: page.page_box[0], y: page.page_box[1], width: page.page_box[2] - page.page_box[0], height: page.page_box[3] - page.page_box[1] }),
            objects: vec![],
        }).collect() };
        // A no-op scene composes back to the original geometry; export itself refuses it (see the no-change guard test).
        let output_document = compose(&input, &scene).unwrap();
        let page = output_document.get_pages().values().next().copied().unwrap();
        assert_eq!(bounds(&output_document, &inherited(&output_document, page, b"MediaBox").unwrap().unwrap()).unwrap(), [10., 20., 250., 360.]);
        assert_eq!(bounds(&output_document, &inherited(&output_document, page, b"CropBox").unwrap().unwrap()).unwrap(), [20., 30., 220., 330.]);
        assert_eq!(inherited(&output_document, page, b"Rotate").unwrap().unwrap().as_i64().unwrap(), 90);
        let page_dictionary = output_document.get_dictionary(page).unwrap();
        assert!(page_dictionary.get(b"MediaBox").is_err());
        assert!(page_dictionary.get(b"CropBox").is_err());
        assert!(page_dictionary.get(b"Rotate").is_err());
        let annots = page_dictionary.get(b"Annots").unwrap().as_array().unwrap();
        assert_eq!(annots.len(), 3);
        let annotation_rect = output_document.get_dictionary(annots[0].as_reference().unwrap()).unwrap().get(b"Rect").unwrap();
        assert_eq!(bounds(&output_document, annotation_rect).unwrap(), [30., 40., 90., 80.]);
        assert!(output_document.get_dictionary(output_document.trailer.get(b"Root").unwrap().as_reference().unwrap()).unwrap().get(b"Names").is_ok());
        assert!(String::from_utf8_lossy(&output_document.get_page_content(page)).contains("Annotated source"));
    }
    #[test]
    fn overlay_export_preserves_annotated_rotated_offset_pdf_geometry() {
        let temp = TempDir::new().unwrap();
        let input = temp.0.join("annotated-rotated-offset.pdf");
        annotated_rotated_offset_fixture(&input);
        let original = fs::read(&input).unwrap();
        let scene = PdfScene { pages: vec![ScenePage {
            source_index: Some(0), width: 300., height: 200., rotation: 0, crop: None,
            source_rotation: Some(90), source_box: Some(Rect { x: 20., y: 30., width: 200., height: 300. }),
            objects: vec![object(Kind::Text)],
        }] };
        let outcome = export(&ExportRequest { paths: vec![input.clone()], scene, output_location: OutputLocation::AlongsideInput, output_path: None, sections: Default::default() }, input.clone());
        assert!(outcome.error().is_none(), "overlay export failed: {:?}", outcome.error());
        let output = outcome.output_paths().first().unwrap();
        let output_document = Document::load(output).unwrap();
        let page_id = output_document.get_pages().values().next().copied().unwrap();
        assert_eq!(bounds(&output_document, &inherited(&output_document, page_id, b"MediaBox").unwrap().unwrap()).unwrap(), [10., 20., 250., 360.]);
        assert_eq!(bounds(&output_document, &inherited(&output_document, page_id, b"CropBox").unwrap().unwrap()).unwrap(), [20., 30., 220., 330.]);
        for (key, expected) in [(b"BleedBox".as_slice(), [22., 32., 218., 328.]), (b"TrimBox", [24., 34., 216., 326.]), (b"ArtBox", [26., 36., 214., 324.])] {
            assert_eq!(bounds(&output_document, &inherited(&output_document, page_id, key).unwrap().unwrap()).unwrap(), expected);
        }
        assert_eq!(inherited(&output_document, page_id, b"Rotate").unwrap().unwrap().as_i64().unwrap(), 90);
        let page_dictionary = output_document.get_dictionary(page_id).unwrap();
        assert!(page_dictionary.get(b"MediaBox").is_err());
        assert!(page_dictionary.get(b"CropBox").is_err());
        assert!(page_dictionary.get(b"Rotate").is_err());
        assert_eq!(page_dictionary.get(b"Annots").unwrap().as_array().unwrap().len(), 3);
        let catalog_id = output_document.trailer.get(b"Root").unwrap().as_reference().unwrap();
        for key in [b"Names".as_slice(), b"Outlines", b"AcroForm"] { assert!(output_document.get_dictionary(catalog_id).unwrap().has(key)); }
        assert!(matches!(page_dictionary.get(b"Contents").unwrap(), Object::Array(values) if values.len() == 2));
        let content_bytes = output_document.get_page_content(page_id);
        let content = String::from_utf8_lossy(&content_bytes);
        assert!(content.contains("Annotated source"));
        assert!(content.contains("/SceneTextNotoSansRegular 18 Tf"));
        assert!(content.contains("0 1 1 0 20 30 cm"));
        assert_eq!(fs::read(&input).unwrap(), original);
    }
    #[test]
    fn exports_preserve_original_and_existing_outputs_byte_for_byte() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); fixture(&input);
        let original=fs::read(&input).unwrap();
        let occupied=temp.0.join("source-edited.pdf"); fs::write(&occupied,b"existing output").unwrap();
        let request=ExportRequest {paths:vec![input.clone()],scene:scene(),output_location: OutputLocation::AlongsideInput, output_path: None, sections: Default::default()};
        let a=export(&request,input.clone()); let b=export(&request,input.clone());
        assert!(a.error().is_none(),"{:?}",a.error()); assert!(b.error().is_none());
        assert_eq!(a.output_paths(),[temp.0.join("source-edited-1.pdf")]); assert_eq!(b.output_paths(),[temp.0.join("source-edited-2.pdf")]);
        assert_eq!(fs::read(&input).unwrap(),original); assert_eq!(fs::read(occupied).unwrap(),b"existing output");
        assert_eq!(Document::load(&a.output_paths()[0]).unwrap().get_pages().len(),3);
    }
    #[test]
    fn exports_name_the_first_copy_edited_then_number_later_ones() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); fixture(&input);
        let request=ExportRequest {paths:vec![input.clone()],scene:scene(),output_location:OutputLocation::AlongsideInput, output_path: None, sections: Default::default()};
        let names=(0..3).map(|_| {
            let outcome=export(&request,input.clone());
            assert!(outcome.error().is_none(),"{:?}",outcome.error());
            outcome.output_paths()[0].file_name().unwrap().to_string_lossy().into_owned()
        }).collect::<Vec<_>>();
        assert_eq!(names,["source-edited.pdf","source-edited-1.pdf","source-edited-2.pdf"]);
        let mut left=fs::read_dir(&temp.0).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect::<Vec<_>>();
        left.sort();
        assert_eq!(left,["source-edited-1.pdf","source-edited-2.pdf","source-edited.pdf","source.pdf"]);
    }
    #[test]
    fn export_honours_a_custom_folder_and_writes_nothing_beside_the_input() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); fixture(&input);
        let original=fs::read(&input).unwrap();
        let folder=temp.0.join("For client"); fs::create_dir(&folder).unwrap();
        let request=ExportRequest {paths:vec![input.clone()],scene:scene(),output_location:OutputLocation::CustomFolder(folder.clone()), output_path: None, sections: Default::default()};
        let outcome=export(&request,input.clone());
        assert!(outcome.error().is_none(),"{:?}",outcome.error());
        assert_eq!(outcome.output_paths(),[folder.join("source-edited.pdf")]);
        assert_eq!(fs::read_dir(&folder).unwrap().count(),1);
        let beside=fs::read_dir(&temp.0).unwrap().map(|entry| entry.unwrap().file_name()).collect::<HashSet<_>>();
        assert_eq!(beside,HashSet::from(["source.pdf".into(),"For client".into()]));
        assert_eq!(fs::read(&input).unwrap(),original);
    }
    #[test]
    fn failed_exports_leave_no_output_or_temporary_file() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); fixture(&input);
        let original=fs::read(&input).unwrap();
        let folder=temp.0.join("out"); fs::create_dir(&folder).unwrap();
        let mut broken=scene(); broken.pages[1].objects[0].text="unsupported \u{1f600}".into();
        for location in [OutputLocation::AlongsideInput,OutputLocation::CustomFolder(folder.clone())] {
            let outcome=export(&ExportRequest {paths:vec![input.clone()],scene:broken.clone(),output_location:location, output_path: None, sections: Default::default()},input.clone());
            assert!(outcome.error().is_some()); assert!(outcome.output_paths().is_empty());
        }
        // A destination that can't be written fails after the scene is built, and must leave no claim or temporary file behind.
        let missing=temp.0.join("gone");
        let outcome=export(&ExportRequest {paths:vec![input.clone()],scene:scene(),output_location:OutputLocation::CustomFolder(missing.clone()), output_path: None, sections: Default::default()},input.clone());
        assert!(outcome.error().is_some()); assert!(!missing.exists());
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&folder,fs::Permissions::from_mode(0o555)).unwrap();
            let outcome=export(&ExportRequest {paths:vec![input.clone()],scene:scene(),output_location:OutputLocation::CustomFolder(folder.clone()), output_path: None, sections: Default::default()},input.clone());
            fs::set_permissions(&folder,fs::Permissions::from_mode(0o755)).unwrap();
            assert!(outcome.error().is_some());
        }
        assert_eq!(fs::read_dir(&folder).unwrap().count(),0);
        let beside=fs::read_dir(&temp.0).unwrap().map(|entry| entry.unwrap().file_name()).collect::<HashSet<_>>();
        assert_eq!(beside,HashSet::from(["source.pdf".into(),"out".into()]));
        assert_eq!(fs::read(&input).unwrap(),original);
    }
    #[test]
    fn export_detail_summarises_kept_removed_rotated_and_mirrored_pages() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); fixture(&input);
        let outcome=export(&ExportRequest {paths:vec![input.clone()],scene:scene(),output_location:OutputLocation::AlongsideInput, output_path: None, sections: Default::default()},input.clone());
        assert_eq!(outcome.detail,"2 pages kept, 0 removed, 2 rotated, 0 mirrored, 1 blank added.");
        let mut one=scene(); one.pages.truncate(1);
        let outcome=export(&ExportRequest {paths:vec![input.clone()],scene:one,output_location:OutputLocation::AlongsideInput, output_path: None, sections: Default::default()},input.clone());
        assert_eq!(outcome.detail,"1 page kept, 1 removed, 1 rotated, 0 mirrored.");
    }
    #[test]
    fn the_same_scene_exports_byte_identical_copies() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); fixture(&input);
        let request=ExportRequest {paths:vec![input.clone()],scene:scene(),output_location:OutputLocation::AlongsideInput, output_path: None, sections: Default::default()};
        let first=export(&request,input.clone()); let second=export(&request,input.clone());
        assert_ne!(first.output_paths(),second.output_paths());
        assert_eq!(fs::read(&first.output_paths()[0]).unwrap(),fs::read(&second.output_paths()[0]).unwrap());
    }
    #[test]
    fn export_drops_pages_omitted_from_the_scene_and_keeps_the_remaining_order() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("labelled.pdf"); labelled_pages_fixture(&input,&["Page A","Page B","Page C","Page D"]);
        let original=fs::read(&input).unwrap();
        let keep=|index| ScenePage{source_index:Some(index),width:612.,height:792.,rotation:0,crop:None,source_rotation:Some(0),source_box:None,objects:vec![]};
        let outcome=export(&ExportRequest{paths:vec![input.clone()],scene:PdfScene{pages:[0,2,3].map(keep).to_vec()},output_location:OutputLocation::AlongsideInput, output_path: None, sections: Default::default()},input.clone());
        assert!(outcome.error().is_none(),"{:?}",outcome.error());
        let labels=page_labels(&outcome.output_paths()[0]);
        assert_eq!(labels.len(),3);
        for (content,label) in labels.iter().zip(["Page A","Page C","Page D"]) { assert!(content.contains(&format!("({label})")),"{content}"); }
        assert!(labels.iter().all(|content| !content.contains("(Page B)")));
        assert_eq!(fs::read(&input).unwrap(),original);
    }
    #[test]
    fn export_sets_absolute_scene_rotation_and_bakes_source_rotation_into_content() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); fixture(&input);
        let original=fs::read(&input).unwrap();
        let scene=PdfScene{pages:vec![
            ScenePage{source_index:Some(1),width:612.,height:792.,rotation:90,crop:None,source_rotation:Some(0),source_box:None,objects:vec![]},
            ScenePage{source_index:Some(0),width:300.,height:200.,rotation:180,crop:None,source_rotation:Some(90),source_box:None,objects:vec![]},
        ]};
        let outcome=export(&ExportRequest{paths:vec![input.clone()],scene,output_location:OutputLocation::AlongsideInput, output_path: None, sections: Default::default()},input.clone());
        assert!(outcome.error().is_none(),"{:?}",outcome.error());
        let output=Document::load(&outcome.output_paths()[0]).unwrap();
        let pages:Vec<_>=output.get_pages().values().copied().collect();
        assert_eq!(pages.len(),2);
        assert_eq!(inherited(&output,pages[0],b"Rotate").unwrap().unwrap().as_i64().unwrap(),90);
        assert_eq!(bounds(&output,&inherited(&output,pages[0],b"MediaBox").unwrap().unwrap()).unwrap(),[0.,0.,612.,792.]);
        assert_eq!(inherited(&output,pages[1],b"Rotate").unwrap().unwrap().as_i64().unwrap(),180);
        assert_eq!(bounds(&output,&inherited(&output,pages[1],b"MediaBox").unwrap().unwrap()).unwrap(),[0.,0.,300.,200.]);
        assert!(String::from_utf8_lossy(&output.get_page_content(pages[1])).contains("0 1 1 0 -30 -20 cm"));
        assert_eq!(fs::read(&input).unwrap(),original);
    }
    #[test]
    fn export_rejects_a_scene_without_pages_before_output() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); fixture(&input);
        let original=fs::read(&input).unwrap();
        let outcome=export(&ExportRequest{paths:vec![input.clone()],scene:PdfScene{pages:vec![]},output_location:OutputLocation::AlongsideInput, output_path: None, sections: Default::default()},input.clone());
        assert!(outcome.error().is_some_and(|error| error.message.contains("1–2000 pages")),"{:?}",outcome.error());
        assert!(outcome.output_paths().is_empty());
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(),1);
        assert_eq!(fs::read(&input).unwrap(),original);
    }
    #[test]
    fn scene_reservation_does_not_replace_a_competing_destination() {
        let temp=TempDir::new().unwrap();
        let candidate=temp.0.join("source-edited.pdf");
        let reservation=OutputNaming::reserve_named_candidate(&candidate).unwrap().unwrap();
        fs::write(reservation.path(),b"scene bytes").unwrap();
        fs::write(&candidate,b"competing replacement").unwrap();

        assert!(reservation.publish().is_err());
        assert_eq!(fs::read(candidate).unwrap(),b"competing replacement");
    }
    #[test]
    fn validation_fails_closed_before_writing_outputs() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); fixture(&input);
        let mut s=scene(); s.pages[0].crop.as_mut().unwrap().x=-1.; assert!(compose(&input,&s).is_err());
        let mut s=scene(); s.pages[0].objects[0].opacity=f32::NAN; assert!(compose(&input,&s).is_err());
        let mut s=scene(); s.pages[0].source_index=Some(90); assert!(compose(&input,&s).is_err());
        let mut s=scene(); s.pages[1].objects[0].text="unsupported \u{1f600}".into();
        let result=export(&ExportRequest {paths:vec![],scene:s,output_location: OutputLocation::AlongsideInput, output_path: None, sections: Default::default()},input.clone());
        assert!(result.error().is_some()); assert!(result.output_paths().is_empty());
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(),1);
        assert!(compose(&input,&PdfScene{pages:vec![]}).is_err());
    }
    #[test]
    fn preview_pages_rejects_empty_duplicate_out_of_range_and_oversized_sets() {
        let cases=[
            (vec![], "Preview requests must include 1–3 page indices"),
            (vec![0, 0], "Preview page indices must be unique"),
            (vec![3], "Preview page outside scene"),
            (vec![0, 1, 2, 0], "Preview requests must include 1–3 page indices"),
        ];
        for (page_indices,message) in cases {
            let result=preview_pages(&PreviewPagesRequest { path: PathBuf::from("missing.pdf"), scene: scene(), page_indices, sections: Default::default() });
            assert_eq!(result.err().as_deref(),Some(message));
        }
    }

    #[test]
    fn preview_pages_returns_ordered_indexed_previews_from_one_normalized_scene() {
        let _guard = crate::kit::lock_process_env();
        let renderer=match renderer() { Ok(r)=>r,Err(e)=>{if std::env::var_os("TOOLBOX_REQUIRE_PDF_RENDERER").is_some(){panic!("{e}")}; eprintln!("Renderer unavailable: {e}");return;} };
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); fixture(&input);
        let before=fs::read(&input).unwrap();
        let request=PreviewPagesRequest { path:input.clone(), scene:scene(), page_indices:vec![2,0,1], sections: Default::default() };
        let previews=preview_pages(&request).unwrap();
        assert_eq!(previews.iter().map(|preview| preview.page_index).collect::<Vec<_>>(),vec![2,0,1]);
        assert!(previews.iter().all(|preview| preview.preview.data_url.starts_with("data:image/png;base64,")));
        assert_eq!(fs::read(&input).unwrap(),before);

        let normalized=temp.0.join("normalized.pdf"); compose(&input,&request.scene).unwrap().save(&normalized).unwrap();
        let render_temp=TempDir::new().unwrap();
        for indexed in &previews {
            let expected=render(&normalized,indexed.page_index,&renderer,&render_temp).unwrap();
            assert_eq!(indexed.preview.data_url,expected.data_url,"page {} differs from the normalized scene render",indexed.page_index);
        }
    }

    #[test]
    fn preview_and_export_are_pixel_identical_and_temp_storage_is_cleaned() {
        let _guard = crate::kit::lock_process_env();
        let renderer=match renderer() { Ok(r)=>r,Err(e)=>{if std::env::var_os("TOOLBOX_REQUIRE_PDF_RENDERER").is_some(){panic!("{e}")}; eprintln!("Renderer unavailable: {e}");return;} };
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); fixture(&input);
        let before=fs::read(&input).unwrap();
        let request=ExportRequest{paths:vec![input.clone()],scene:scene(),output_location: OutputLocation::AlongsideInput, output_path: None, sections: Default::default()};
        let exported=export(&request,input.clone()); assert!(exported.error().is_none());
        for index in 0..3 {
            let preview=preview(&PreviewRequest{path:input.clone(),scene:scene(),page_index:index,sections:Default::default()}).unwrap();
            let actual=render(&exported.output_paths()[0],index,&renderer,&temp).unwrap();
            assert_eq!(preview.data_url,actual.data_url,"page {index} differs between preview and export");
        }
        assert_eq!(fs::read(&input).unwrap(),before);
        let directory={let guard=TempDir::new().unwrap();let p=guard.0.clone();fs::write(p.join("private.pdf"),b"temporary").unwrap();p};
        assert!(!directory.exists());
        // Removing all source marks must visibly differ: negative control for the pixel oracle.
        let mut bare=scene();bare.pages[0].objects.clear();
        assert_ne!(preview(&PreviewRequest{path:input.clone(),scene:bare,page_index:0,sections:Default::default()}).unwrap().data_url,
            preview(&PreviewRequest{path:input,scene:scene(),page_index:0,sections:Default::default()}).unwrap().data_url);
    }

    #[test]
    fn renders_shape_variants_and_rotated_watermarks() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); fixture(&input);
        let variants=[Shape::Square,Shape::Round,Shape::Triangle,Shape::Line,Shape::DottedLine,Shape::ArrowLeft,Shape::ArrowRight,Shape::ArrowUp,Shape::ArrowDown];
        let mut objects=variants.into_iter().map(|shape| { let mut value=object(Kind::Shape); value.shape=Some(shape); value }).collect::<Vec<_>>();
        let mut watermark=object(Kind::Watermark); watermark.watermark.as_mut().unwrap().rotation=90.; objects.push(watermark);
        let mut model=scene(); model.pages[0].objects=objects;
        let composed=compose(&input,&model).unwrap(); let page_id=composed.get_pages().values().next().copied().unwrap();
        let content_bytes=composed.get_page_content(page_id); let content=String::from_utf8_lossy(&content_bytes);
        assert!(content.contains("[3 4] 0 d"));
        assert!(content.contains("0 1 -1 0 "), "{content}");
        assert!(content.contains("m ")); assert!(content.contains(" h f"));
    }

    #[test]
    fn image_signatures_preserve_transparency_and_use_a_local_xobject() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); fixture(&input);
        let signature=temp.0.join("signature.png");
        let mut image=image::RgbaImage::new(4,4);
        for pixel in image.pixels_mut() { *pixel=image::Rgba([0,0,0,0]); }
        image.put_pixel(1,1,image::Rgba([0,0,0,255])); image.save(&signature).unwrap();
        let mut value=object(Kind::Signature); value.signature_mode=Some(SignatureMode::Image); value.signature_path=Some(signature);
        let mut model=scene(); model.pages[0].objects=vec![value];
        let composed=compose(&input,&model).unwrap(); let page_id=composed.get_pages().values().next().copied().unwrap();
        let content_bytes=composed.get_page_content(page_id); let content=String::from_utf8_lossy(&content_bytes); assert!(content.contains("/Sig0 Do"));
        let has_mask=composed.objects.values().filter_map(|value|value.as_stream().ok()).any(|stream|stream.dict.has(b"SMask"));
        assert!(has_mask);
    }

    fn signature_object(mode: SignatureMode, path: Option<PathBuf>, text: &str) -> SceneObject {
        SceneObject { rect: Rect{x:100.,y:200.,width:200.,height:200.}, text: text.into(), color: "#1A3A8F".into(), opacity: 1.,
            signature_mode: Some(mode), signature_path: path, ..object(Kind::Signature) }
    }

    #[test]
    fn typed_signatures_use_the_one_bundled_script_font_with_noto_sans_fallback() {
        let pacifico = fontdue::Font::from_bytes(SIGNATURE_FONT, fontdue::FontSettings::default()).unwrap();
        let noto = fontdue::Font::from_bytes(NOTO_SANS_REGULAR, fontdue::FontSettings::default()).unwrap();
        assert!(!pacifico.has_glyph('α') && noto.has_glyph('α'), "fallback probe must be missing from the script font only");
        let ink = [0.1, 0.23, 0.56];
        let latin = signature_text_image("A. Local", ink).unwrap();
        assert_eq!(latin.rgb, signature_text_image("A. Local", ink).unwrap().rgb, "typed signatures must rasterize deterministically");
        assert_eq!(latin.alpha, signature_text_image("A. Local", ink).unwrap().alpha);
        let fallback = signature_text_image("A. Localα", ink).unwrap();
        assert!(fallback.width > latin.width, "the Noto Sans fallback glyph must be drawn");
        let missing = signature_text_image("A. Ա", ink).err().unwrap();
        assert_eq!(missing.kind, crate::kit::contracts::ErrorKind::InvalidInput);
        assert_eq!(missing.message, "The signature font can't show this character: “Ա” (U+0531). Remove it to export this signature.");
        assert_eq!(signature_text_image("  ", ink).err().unwrap().message, "Type a name for the signature before placing it.");

        let temp = TempDir::new().unwrap(); let input = temp.0.join("source.pdf"); fixture(&input);
        let mut model = scene(); model.pages[0].objects = vec![signature_object(SignatureMode::Text, None, "A. Local")];
        let composed = compose(&input, &model).unwrap();
        let page_id = composed.get_pages().values().next().copied().unwrap();
        let content = String::from_utf8_lossy(&composed.get_page_content(page_id)).into_owned();
        assert!(content.contains("/Sig0 Do") && !content.contains(" Tj"), "typed signature must be an image, not text: {content}");
        let xobjects = composed.get_dictionary(page_id).unwrap().get(b"Resources").unwrap().as_dict().unwrap().get(b"XObject").unwrap().as_dict().unwrap();
        let signature = composed.get_object(xobjects.get(b"Sig0").unwrap().as_reference().unwrap()).unwrap().as_stream().unwrap();
        assert_eq!(signature.dict.get(b"Subtype").unwrap().as_name().unwrap(), b"Image");
        assert!(signature.dict.has(b"SMask"), "typed signature lost transparency");
    }

    #[test]
    fn signature_pictures_accept_png_and_jpg_by_content_and_refuse_everything_else_before_output() {
        let temp = TempDir::new().unwrap(); let input = temp.0.join("source.pdf"); fixture(&input);
        let png = temp.0.join("ink.png");
        image::RgbaImage::from_fn(8, 4, |x, _| image::Rgba([0, 0, 0, if x < 4 { 255 } else { 0 }])).save(&png).unwrap();
        let jpg = temp.0.join("scan.jpg");
        image::RgbImage::from_pixel(8, 4, image::Rgb([20, 20, 20])).save(&jpg).unwrap();
        let png_signature = signature_image(&png).unwrap();
        assert_eq!((png_signature.width, png_signature.height), (8, 4));
        assert!(png_signature.alpha.is_some(), "PNG transparency must be kept");
        assert!(signature_image(&jpg).unwrap().alpha.is_none());
        let renamed = temp.0.join("scan.png"); fs::copy(&jpg, &renamed).unwrap();
        assert!(signature_image(&renamed).is_ok(), "formats are sniffed from content, not the name");

        let webp = temp.0.join("ink.webp"); image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 0, 255])).save(&webp).unwrap();
        let tiff = temp.0.join("ink.tiff"); image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 0, 255])).save(&tiff).unwrap();
        let disguised = temp.0.join("disguised.png"); fs::copy(&tiff, &disguised).unwrap();
        let not_image = temp.0.join("notes.jpg"); fs::write(&not_image, b"not a picture").unwrap();
        let huge = temp.0.join("huge.png"); fs::write(&huge, crate::kit::common::limits::fixtures::png_header(5000, 5000)).unwrap();
        let before: Vec<_> = fs::read_dir(&temp.0).unwrap().map(|entry| entry.unwrap().path()).collect();
        for (path, kind, message) in [
            (&webp, crate::kit::contracts::ErrorKind::InvalidInput, "ink.webp can't be used. Choose a PNG or JPG. A transparent PNG looks best."),
            (&tiff, crate::kit::contracts::ErrorKind::InvalidInput, "ink.tiff can't be used. Choose a PNG or JPG. A transparent PNG looks best."),
            (&disguised, crate::kit::contracts::ErrorKind::InvalidInput, "disguised.png can't be used. Choose a PNG or JPG. A transparent PNG looks best."),
            (&not_image, crate::kit::contracts::ErrorKind::InvalidInput, "notes.jpg can't be used. Choose a PNG or JPG. A transparent PNG looks best."),
            (&huge, crate::kit::contracts::ErrorKind::LimitExceeded { limit: crate::kit::contracts::LimitKind::Decompression }, "This image expands to more than 80 MB when opened, which is over the limit. It may be damaged or unsafe to open."),
        ] {
            assert_eq!(inspect_signature_image(path).unwrap_err(), message);
            let outcome = export_text(&input, text_scene(vec![signature_object(SignatureMode::Image, Some(path.clone()), "")]));
            let error = outcome.error().expect("refused signatures must fail the export");
            assert_eq!((&error.kind, error.message.as_str()), (&kind, message));
        }
        let after: Vec<_> = fs::read_dir(&temp.0).unwrap().map(|entry| entry.unwrap().path()).collect();
        assert_eq!(before, after, "refused signatures must not write output");
    }

    #[test]
    fn signatures_keep_their_proportions_and_can_repeat_across_a_page_range() {
        let temp = TempDir::new().unwrap(); let input = temp.0.join("source.pdf"); page_text_fixture(&input);
        let png = temp.0.join("wide.png");
        image::RgbaImage::from_pixel(40, 20, image::Rgba([255, 0, 0, 255])).save(&png).unwrap();
        let mut initials = signature_object(SignatureMode::Image, Some(png), "");
        initials.page_range = Some("1-2".into());
        let mut doc = compose(&input, &text_scene(vec![initials])).unwrap();
        let pages: Vec<_> = doc.get_pages().values().copied().collect();
        let image_of = |page: ObjectId| {
            let content = String::from_utf8_lossy(&doc.get_page_content(page)).into_owned();
            assert!(content.contains("q 200 0 0 -100 100 350 cm /Sig0 Do Q"), "the 2:1 picture must fit the square box without stretching: {content}");
            doc.get_dictionary(page).unwrap().get(b"Resources").unwrap().as_dict().unwrap().get(b"XObject").unwrap().as_dict().unwrap().get(b"Sig0").unwrap().as_reference().unwrap()
        };
        assert_eq!(image_of(pages[0]), image_of(pages[1]), "repeated signatures share one image");
        assert!(!String::from_utf8_lossy(&doc.get_page_content(pages[2])).contains("Do"), "pages outside the range stay clear");

        let Ok(renderer) = renderer() else { return };
        let path = temp.0.join("scene.pdf"); doc.save(&path).unwrap();
        let preview = render(&path, 1, &renderer, &temp).unwrap();
        let png = base64::engine::general_purpose::STANDARD.decode(preview.data_url.trim_start_matches("data:image/png;base64,")).unwrap();
        let pixels = image::load_from_memory(&png).unwrap().to_rgb8();
        let scale = pixels.width() as f32 / 612.;
        let at = |x: f32, y: f32| pixels.get_pixel((x * scale) as u32, (y * scale) as u32).0;
        assert_eq!(at(200., 300.), [255, 0, 0], "signature ink must render inside the fitted area");
        assert_eq!(at(200., 220.), [255, 255, 255], "the fitted picture must not stretch into the empty band");
        assert_eq!(at(200., 380.), [255, 255, 255]);
    }

    #[test]
    fn scene_allocates_overlay_resources_around_inherited_names() {
        let temp = TempDir::new().unwrap();
        let input = temp.0.join("inherited-collisions.pdf");
        let signature = temp.0.join("signature.png");
        inherited_collision_fixture(&input);
        let image = image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 0, 255]));
        image.save(&signature).unwrap();

        let mut signature_object = object(Kind::Signature);
        signature_object.signature_mode = Some(SignatureMode::Image);
        signature_object.signature_path = Some(signature);
        let model = PdfScene { pages: vec![ScenePage {
            source_index: Some(0), width: 612., height: 792., rotation: 0,
            crop: Some(Rect { x: 0., y: 0., width: 500., height: 700. }),
            source_rotation: None, source_box: None,
            objects: vec![object(Kind::Watermark), signature_object, SceneObject { signature_mode: Some(SignatureMode::Text), ..object(Kind::Signature) }],
        }] };

        let composed = compose(&input, &model).unwrap();
        let page = composed.get_pages().values().next().copied().unwrap();
        let content = String::from_utf8_lossy(&composed.get_page_content(page)).into_owned();
        assert!(content.contains("/SceneTextNotoSansRegular1 50 Tf"), "font resource collision: {content}");
        assert!(content.contains("/S02 gs") && content.contains("/S12 gs"), "state resource collisions: {content}");
        assert!(content.contains("/Original2 Do") && content.contains("/Sig12 Do"), "xobject resource collisions: {content}");

        let resources = composed.get_dictionary(page).unwrap().get(b"Resources").unwrap().as_dict().unwrap();
        let fonts = resources.get(b"Font").unwrap().as_dict().unwrap();
        let xobjects = resources.get(b"XObject").unwrap().as_dict().unwrap();
        let states = resources.get(b"ExtGState").unwrap().as_dict().unwrap();
        for name in ["SceneFont", "SceneFont1", "SceneFont2", "SceneTextNotoSansRegular1"] { assert!(fonts.has(name.as_bytes())); }
        for name in ["Original", "Original1", "Original2", "Sig1", "Sig11", "Sig12"] { assert!(xobjects.has(name.as_bytes())); }
        for name in ["S0", "S01", "S02", "S1", "S11", "S12"] { assert!(states.has(name.as_bytes())); }
    }

    const BUNDLED_PDFTOTEXT_CHILD: &str = "TOOLBOX_TEST_BUNDLED_PDFTOTEXT_CHILD";

    // PATH and the resource root are changed only in a child test process, because other tests spawn helpers by name.
    #[cfg(unix)]
    #[test]
    fn text_preview_uses_the_bundled_pdftotext_when_path_is_empty() {
        if std::env::var_os(BUNDLED_PDFTOTEXT_CHILD).is_none() {
            use std::os::unix::fs::PermissionsExt;
            let _guard = crate::kit::lock_process_env();
            let temp = TempDir::new().unwrap();
            let pdf_bin = temp.0.join("Resources").join("pdf-bin");
            fs::create_dir_all(&pdf_bin).unwrap();
            let stand_in = pdf_bin.join("pdftotext");
            // Shell built-ins only: the child runs with an empty PATH.
            fs::write(&stand_in, "#!/bin/sh\ncase \"$1\" in -h) exit 0;; esac\nprintf '%s\\n' '<doc><page width=\"300\" height=\"200\"><word xMin=\"10\" yMin=\"20\" xMax=\"60\" yMax=\"30\">BundledStandIn</word></page></doc>'\n").unwrap();
            fs::set_permissions(&stand_in, fs::Permissions::from_mode(0o755)).unwrap();
            let child = Command::new(std::env::current_exe().unwrap())
                .args(["kit::pdf::scene::tests::text_preview_uses_the_bundled_pdftotext_when_path_is_empty", "--exact", "--nocapture", "--test-threads=1"])
                .env(BUNDLED_PDFTOTEXT_CHILD, "1")
                .env("PATH", "")
                .env("TOOLBOX_RESOURCE_ROOT", temp.0.join("Resources"))
                .env_remove("TOOLBOX_PDFTOTEXT_PATH")
                .output()
                .unwrap();
            let report = format!("{}{}", String::from_utf8_lossy(&child.stdout), String::from_utf8_lossy(&child.stderr));
            assert!(child.status.success() && report.contains("1 passed"), "{report}");
            return;
        }
        assert_eq!(std::env::var_os("PATH").unwrap_or_default(), "");
        let temp = TempDir::new().unwrap();
        let input = temp.0.join("source.pdf");
        fixture(&input);
        let runs = extract_text_runs(&input, 1, 300., 200., Instant::now() + super::super::PDF_TEXT_TIMEOUT).expect("the bundled pdftotext should be found");
        assert_eq!(runs.len(), 1, "runs: {runs:?}");
        assert_eq!(runs[0].text, "BundledStandIn");
        assert_eq!((runs[0].x, runs[0].y, runs[0].width, runs[0].height), (10., 20., 50., 10.));
    }

    #[test]
    fn extracts_selectable_text_runs_when_the_local_poppler_helper_is_available() {
        if text_extractor().is_none() { return; }
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); fixture(&input);
        let runs=extract_text_runs(&input,1,300.,200.,Instant::now() + super::super::PDF_TEXT_TIMEOUT).unwrap();
        assert!(runs.iter().any(|run|run.text.contains("Original")),"runs: {runs:?}");
    }
    #[test]
    fn scene_preserves_catalog_structures_and_page_annotations() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("structured.pdf"); fixture(&input);
        let mut source=Document::load(&input).unwrap();
        let pages:Vec<_>=source.get_pages().values().copied().collect();
        source.get_dictionary_mut(pages[0]).unwrap().set("MediaBox", array(&[0.,0.,200.,300.]));
        source.get_dictionary_mut(pages[0]).unwrap().set("CropBox", array(&[0.,0.,200.,300.]));
        source.get_dictionary_mut(pages[0]).unwrap().set("Rotate", 0);
        let annotation=source.add_object(dictionary! {"Type"=>"Annot", "Subtype"=>"Link", "Rect"=>array(&[20.,20.,80.,50.]), "A"=>dictionary! {"S"=>"URI", "URI"=>"https://example.invalid"}});
        let widget=source.add_object(dictionary! {"Type"=>"Annot", "Subtype"=>"Widget", "FT"=>"Tx", "Rect"=>array(&[90.,20.,180.,50.])});
        source.get_dictionary_mut(pages[1]).unwrap().set("Annots",vec![Object::Reference(annotation),Object::Reference(widget)]);
        let metadata=source.add_object(Stream::new(dictionary! {"Type"=>"Metadata", "Subtype"=>"XML"}, b"<xmpmeta>fixture</xmpmeta>".to_vec()));
        let names=source.add_object(dictionary! {"Dests"=>dictionary! {"fixture"=>Object::Reference(pages[0])}});
        let outlines=source.add_object(dictionary! {"Type"=>"Outlines", "Count"=>0});
        let structure=source.add_object(dictionary! {"Type"=>"StructTreeRoot", "K"=>vec![]});
        let acro_form=source.add_object(dictionary! {"Fields"=>vec![Object::Reference(widget)]});
        let catalog_id=source.trailer.get(b"Root").unwrap().as_reference().unwrap();
        let catalog=source.get_dictionary_mut(catalog_id).unwrap();
        catalog.set("Metadata",metadata); catalog.set("Names",names); catalog.set("Outlines",outlines); catalog.set("StructTreeRoot",structure); catalog.set("AcroForm",acro_form);
        source.save(&input).unwrap();
        let original=fs::read(&input).unwrap();
        let scene=PdfScene { pages: pages.iter().enumerate().map(|(index,_)| ScenePage { source_index:Some(index), width:if index==0 {200.} else {612.}, height:if index==0 {300.} else {792.}, rotation:0, crop:None, source_rotation:None, source_box:None, objects:vec![] }).collect() };
        let composed=compose(&input,&scene).unwrap();
        let catalog=composed.get_dictionary(catalog_id).unwrap();
        for key in [b"Metadata".as_slice(),b"Names",b"Outlines",b"StructTreeRoot",b"AcroForm"] { assert!(catalog.has(key),"catalog lost {key:?}"); }
        let output_pages:Vec<_>=composed.get_pages().values().copied().collect();
        assert_eq!(output_pages, pages);
        let annots=composed.get_dictionary(output_pages[1]).unwrap().get(b"Annots").unwrap().as_array().unwrap();
        assert_eq!(annots.len(),2);
        assert_eq!(fs::read(&input).unwrap(),original);
    }

    #[test]
    fn tagged_pure_page_reorder_preserves_existing_objects_and_references() {
        let temp = TempDir::new().unwrap();
        let input = temp.0.join("tagged-reorder.pdf");
        tagged_reorder_fixture(&input);
        let original_bytes = fs::read(&input).unwrap();
        let original = Document::load(&input).unwrap();
        let pages: Vec<_> = original.get_pages().values().copied().collect();
        let catalog = original.trailer.get(b"Root").unwrap().as_reference().unwrap();
        let pages_root = original.get_dictionary(catalog).unwrap().get(b"Pages").unwrap().as_reference().unwrap();
        let scene = PdfScene { pages: vec![
            ScenePage {
                source_index: Some(1), width: 612., height: 792., rotation: 0, crop: None,
                source_rotation: Some(0), source_box: Some(Rect { x: 0., y: 0., width: 612., height: 792. }), objects: vec![],
            },
            ScenePage {
                source_index: Some(0), width: 300., height: 200., rotation: 0, crop: None,
                source_rotation: Some(90), source_box: Some(Rect { x: 20., y: 30., width: 200., height: 300. }), objects: vec![],
            },
        ] };

        let composed = compose(&input, &scene).expect("tagged pure reorder should compose");
        let reordered: Vec<_> = composed.get_pages().values().copied().collect();
        assert_eq!(reordered, vec![pages[1], pages[0]]);
        assert_only_page_tree_order_changed(&composed, &original, pages_root, &reordered);
        assert_eq!(fs::read(&input).unwrap(), original_bytes);
    }

    #[test]
    fn scene_rejects_nested_page_tree_before_output() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("nested.pdf"); nested_fixture(&input);
        let scene=PdfScene { pages:vec![ScenePage { source_index:Some(0), width:300., height:200., rotation:0, crop:None, source_rotation:None, source_box:None, objects:vec![] }, ScenePage { source_index:Some(1), width:612., height:792., rotation:0, crop:None, source_rotation:None, source_box:None, objects:vec![] }] };
        let result=export(&ExportRequest {paths:vec![input.clone()],scene,output_location: OutputLocation::AlongsideInput, output_path: None, sections: Default::default()},input.clone());
        assert!(result.error().is_some_and(|error| error.message.contains("nested")));
        assert!(result.output_paths().is_empty());
        assert!(!input.with_file_name("nested-edited.pdf").exists());
    }
    #[test]
    fn scene_rejects_annotation_crop_before_output() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("annotated.pdf"); fixture(&input);
        let mut source=Document::load(&input).unwrap(); let page=source.get_pages().values().next().copied().unwrap();
        let annotation=source.add_object(dictionary! {"Type"=>"Annot", "Subtype"=>"Text", "Rect"=>array(&[20.,20.,80.,50.])});
        source.get_dictionary_mut(page).unwrap().set("Annots",vec![Object::Reference(annotation)]); source.save(&input).unwrap();
        let scene=PdfScene { pages:vec![ScenePage { source_index:Some(0), width:300., height:200., rotation:0, crop:Some(Rect{x:0.,y:0.,width:100.,height:100.}), source_rotation:None, source_box:None, objects:vec![] }] };
        let result=export(&ExportRequest {paths:vec![input.clone()],scene,output_location: OutputLocation::AlongsideInput, output_path: None, sections: Default::default()},input.clone());
        assert!(result.error().is_some_and(|error| error.message.contains("annotation")));
        assert!(result.output_paths().is_empty());
        assert!(!input.with_file_name("annotated-edited.pdf").exists());
    }
    #[test]
    fn scene_rejects_digitally_signed_pdf_before_output() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("signed.pdf"); fixture(&input);
        let mut source=Document::load(&input).unwrap();
        let signature=source.add_object(dictionary! {"Type"=>"Annot", "Subtype"=>"Widget", "FT"=>"Sig", "Rect"=>array(&[10.,10.,80.,30.])});
        let acro_form=source.add_object(dictionary! {"Fields"=>vec![Object::Reference(signature)]});
        let catalog_id=source.trailer.get(b"Root").unwrap().as_reference().unwrap();
        source.get_dictionary_mut(catalog_id).unwrap().set("AcroForm",acro_form);
        source.save(&input).unwrap();
        let scene=PdfScene { pages:vec![
            ScenePage {source_index:Some(0),width:300.,height:200.,rotation:0,crop:None,source_rotation:None,source_box:None,objects:vec![]},
            ScenePage {source_index:Some(1),width:612.,height:792.,rotation:0,crop:None,source_rotation:None,source_box:None,objects:vec![]},
        ] };
        let result=export(&ExportRequest {paths:vec![input.clone()],scene,output_location: OutputLocation::AlongsideInput, output_path: None, sections: Default::default()},input.clone());
        assert!(result.error().is_some_and(|error| error.message.contains("Digitally signed")));
        assert!(result.output_paths().is_empty());
        assert!(!input.with_file_name("signed-edited.pdf").exists());
    }
    #[test]
    fn inspect_reports_whether_the_source_holds_a_signature_value() {
        let temp=TempDir::new().unwrap();
        let plain=temp.0.join("plain.pdf"); fixture(&plain);
        let signed=temp.0.join("signed.pdf"); fixture(&signed);
        let mut source=Document::load(&signed).unwrap(); crate::kit::pdf::tests::sign(&mut source); source.save(&signed).unwrap();
        let empty_field=temp.0.join("empty-field.pdf"); fixture(&empty_field);
        let mut source=Document::load(&empty_field).unwrap(); crate::kit::pdf::tests::add_signature_field(&mut source, None); source.save(&empty_field).unwrap();
        assert!(inspect(&signed).unwrap().signed);
        assert!(!inspect(&plain).unwrap().signed);
        assert!(!inspect(&empty_field).unwrap().signed);
        assert!(super::super::metadata::inspect(&signed).unwrap().signed);
        assert!(!super::super::metadata::inspect(&plain).unwrap().signed);
    }
    #[test]
    fn scene_export_of_a_signed_source_succeeds_warns_and_writes_an_unsigned_copy() {
        use crate::kit::contracts::{OutcomeWarning, WarningKind};
        let removed=vec![OutcomeWarning { kind: WarningKind::SignatureRemoved, message: crate::kit::pdf::SIGNATURE_REMOVED.into() }];
        let reorder=PdfScene { pages:vec![
            ScenePage {source_index:Some(1),width:612.,height:792.,rotation:0,crop:None,source_rotation:None,source_box:None,objects:vec![]},
            ScenePage {source_index:Some(0),width:300.,height:200.,rotation:0,crop:None,source_rotation:None,source_box:None,objects:vec![]},
        ] };
        let overlay=PdfScene { pages:vec![
            ScenePage {source_index:Some(0),width:300.,height:200.,rotation:0,crop:None,source_rotation:None,source_box:None,objects:vec![object(Kind::Shape),object(Kind::Text)]},
            ScenePage {source_index:Some(1),width:612.,height:792.,rotation:0,crop:None,source_rotation:None,source_box:None,objects:vec![]},
        ] };
        for (name,scene) in [("reorder",reorder),("overlay",overlay)] {
            let temp=TempDir::new().unwrap();
            let signed=temp.0.join("signed.pdf"); fixture(&signed);
            let mut source=Document::load(&signed).unwrap(); crate::kit::pdf::tests::sign(&mut source); source.save(&signed).unwrap();
            let plain=temp.0.join("plain.pdf"); fixture(&plain);
            let signed_bytes=fs::read(&signed).unwrap(); let plain_bytes=fs::read(&plain).unwrap();

            let outcome=export(&ExportRequest {paths:vec![signed.clone()],scene:scene.clone(),output_location:OutputLocation::AlongsideInput, output_path: None, sections: Default::default()},signed.clone());
            assert!(outcome.error().is_none(), "{name}: {:?}", outcome.error());
            assert_eq!(outcome.output_paths(),[temp.0.join("signed-edited.pdf")], "{name}");
            assert_eq!(outcome.warnings,removed, "{name}");
            let copy=Document::load(&outcome.output_paths()[0]).unwrap();
            assert_eq!(copy.get_pages().len(),2, "{name}");
            assert!(!crate::kit::pdf::has_signed_signature_field(&copy), "{name}: the copy still holds a signature value");
            assert_eq!(fs::read(&signed).unwrap(),signed_bytes, "{name}: the signed original changed");

            let outcome=export(&ExportRequest {paths:vec![plain.clone()],scene,output_location:OutputLocation::AlongsideInput, output_path: None, sections: Default::default()},plain.clone());
            assert!(outcome.error().is_none(), "{name}: {:?}", outcome.error());
            assert!(outcome.warnings.is_empty(), "{name}: an unsigned source must not warn");
            assert_eq!(fs::read(&plain).unwrap(),plain_bytes, "{name}: the unsigned original changed");
        }
    }
    #[test]
    fn scene_rejects_catalog_permissions_and_signature_dictionaries_before_output() {
        let cases = [
            ("perms", dictionary! {}, true),
            ("type", dictionary! {"Type"=>"Sig"}, false),
            ("byte-range", dictionary! {"ByteRange"=>vec![0.into(), 10.into(), 20.into(), 30.into()]}, false),
        ];
        for (name, signature_dictionary, permissions) in cases {
            let temp=TempDir::new().unwrap(); let input=temp.0.join(format!("{name}.pdf")); fixture(&input);
            let mut source=Document::load(&input).unwrap();
            let signature=source.add_object(signature_dictionary);
            let catalog_id=source.trailer.get(b"Root").unwrap().as_reference().unwrap();
            if permissions {
                source.get_dictionary_mut(catalog_id).unwrap().set("Perms", dictionary! {"DocMDP"=>Object::Reference(signature)});
            }
            source.save(&input).unwrap();
            let scene=PdfScene { pages:vec![
                ScenePage {source_index:Some(0),width:300.,height:200.,rotation:0,crop:None,source_rotation:None,source_box:None,objects:vec![]},
                ScenePage {source_index:Some(1),width:612.,height:792.,rotation:0,crop:None,source_rotation:None,source_box:None,objects:vec![]},
            ] };
            let result=export(&ExportRequest {paths:vec![input.clone()],scene,output_location: OutputLocation::AlongsideInput, output_path: None, sections: Default::default()},input.clone());
            assert!(result.error().is_some_and(|error| error.message.contains("Digitally signed")), "{name}: {:?}", result.error());
            assert!(result.output_paths().is_empty());
            assert!(!input.with_file_name(format!("{name}-edited.pdf")).exists());
        }
    }
    #[test]
    fn scene_rejects_navigation_geometry_edits_before_output() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("navigation.pdf"); fixture(&input);
        let mut source=Document::load(&input).unwrap();
        let outlines=source.add_object(dictionary! {"Type"=>"Outlines", "Count"=>0});
        let catalog_id=source.trailer.get(b"Root").unwrap().as_reference().unwrap();
        source.get_dictionary_mut(catalog_id).unwrap().set("Outlines", outlines);
        source.save(&input).unwrap();
        let scene=PdfScene { pages:vec![
            ScenePage {source_index:Some(0),width:300.,height:200.,rotation:0,crop:Some(Rect{x:0.,y:0.,width:100.,height:100.}),source_rotation:None,source_box:None,objects:vec![]},
            ScenePage {source_index:Some(1),width:612.,height:792.,rotation:0,crop:None,source_rotation:None,source_box:None,objects:vec![]},
        ] };
        let result=export(&ExportRequest {paths:vec![input.clone()],scene,output_location: OutputLocation::AlongsideInput, output_path: None, sections: Default::default()},input.clone());
        assert!(result.error().is_some_and(|error| error.message.contains("destinations")), "{:?}", result.error());
        assert!(result.output_paths().is_empty());
        assert!(!input.with_file_name("navigation-edited.pdf").exists());
    }

    #[test]
    fn scene_accepts_navigation_when_only_the_source_page_box_has_an_offset() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("box-origin-navigation.pdf"); fixture(&input);
        let mut source=Document::load(&input).unwrap();
        let outlines=source.add_object(dictionary! {"Type"=>"Outlines", "Count"=>0});
        let pages:Vec<_>=source.get_pages().values().copied().collect();
        source.get_dictionary_mut(pages[0]).unwrap().set("Rotate", 0);
        let catalog_id=source.trailer.get(b"Root").unwrap().as_reference().unwrap();
        source.get_dictionary_mut(catalog_id).unwrap().set("Outlines", outlines);
        source.save(&input).unwrap();
        let scene=PdfScene { pages:vec![
            ScenePage {source_index:Some(0),width:200.,height:300.,rotation:0,crop:None,source_rotation:None,source_box:None,objects:vec![]},
            ScenePage {source_index:Some(1),width:612.,height:792.,rotation:0,crop:None,source_rotation:None,source_box:None,objects:vec![]},
        ] };
        // The scene is a no-op, so export refuses it; the preflight that accepts navigation is shared with compose.
        assert!(compose(&input,&scene).is_ok());
    }
    #[test]
    fn scene_rejects_tagged_page_removal_before_output() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("tagged.pdf"); fixture(&input);
        let mut source=Document::load(&input).unwrap();
        let pages:Vec<_>=source.get_pages().values().copied().collect();
        let parent_tree=source.add_object(dictionary! {"Nums"=>Object::Array(vec![Object::Integer(0),Object::Dictionary(dictionary! {"Pg"=>Object::Reference(pages[0])})])});
        let structure=source.add_object(dictionary! {"Type"=>"StructTreeRoot","K"=>vec![],"ParentTree"=>parent_tree});
        let catalog_id=source.trailer.get(b"Root").unwrap().as_reference().unwrap();
        source.get_dictionary_mut(catalog_id).unwrap().set("StructTreeRoot",structure);
        source.save(&input).unwrap();
        let scene=PdfScene { pages:vec![ScenePage {source_index:Some(1),width:612.,height:792.,rotation:0,crop:None,source_rotation:None,source_box:None,objects:vec![]}] };
        let result=export(&ExportRequest {paths:vec![input.clone()],scene,output_location: OutputLocation::AlongsideInput, output_path: None, sections: Default::default()},input.clone());
        assert!(result.error().is_some_and(|error| error.message.contains("tagged PDF")), "{:?}", result.error());
        assert!(result.output_paths().is_empty());
        assert!(!input.with_file_name("tagged-edited.pdf").exists());
    }

    #[test]
    fn scene_rejects_tagged_overlay_before_output_but_allows_identity() {
        let temp = TempDir::new().unwrap();
        let input = temp.0.join("tagged-overlay.pdf");
        fixture(&input);
        let mut source = Document::load(&input).unwrap();
        let pages: Vec<_> = source.get_pages().values().copied().collect();
        source.get_dictionary_mut(pages[0]).unwrap().set("StructParents", 0);
        let tagged_content = source.add_object(Stream::new(dictionary! {}, b"/P <</MCID 0>> BDC q 0 0 1 1 re f Q EMC".to_vec()));
        source.get_dictionary_mut(pages[0]).unwrap().set("Contents", tagged_content);
        let parent_tree = source.add_object(dictionary! { "Nums" => Object::Array(vec![Object::Integer(0), Object::Dictionary(dictionary! { "Pg" => Object::Reference(pages[0]) })]) });
        let structure = source.add_object(dictionary! { "Type" => "StructTreeRoot", "K" => vec![], "ParentTree" => parent_tree });
        let catalog_id = source.trailer.get(b"Root").unwrap().as_reference().unwrap();
        source.get_dictionary_mut(catalog_id).unwrap().set("StructTreeRoot", structure);
        source.save(&input).unwrap();
        let identity = PdfScene { pages: vec![
            ScenePage { source_index: Some(0), width: 300., height: 200., rotation: 0, crop: None, source_rotation: Some(90), source_box: Some(Rect { x: 20., y: 30., width: 200., height: 300. }), objects: vec![] },
            ScenePage { source_index: Some(1), width: 612., height: 792., rotation: 0, crop: None, source_rotation: Some(0), source_box: Some(Rect { x: 0., y: 0., width: 612., height: 792. }), objects: vec![] },
        ] };
        assert!(compose(&input, &identity).is_ok());
        let mut overlay = identity;
        overlay.pages[0].objects.push(object(Kind::Text));
        let result = export(&ExportRequest { paths: vec![input.clone()], scene: overlay, output_location: OutputLocation::AlongsideInput, output_path: None, sections: Default::default() }, input.clone());
        assert!(result.error().is_some_and(|error| error.message.contains("tagged PDF")), "{result:?}");
        assert!(result.output_paths().is_empty());
        assert!(!input.with_file_name("tagged-overlay-edited.pdf").exists());
    }

    fn wire_request(input: &Path, sections: serde_json::Value) -> ExportRequest {
        let mut request = serde_json::json!({
            "paths": [input],
            "outputLocation": "alongsideInput",
            "scene": { "pages": [
                { "sourceIndex": 1, "width": 612, "height": 792, "rotation": 0, "crop": null, "objects": [
                    { "kind": "text", "rect": { "x": 72, "y": 150, "width": 360, "height": 40 }, "text": "Staged", "fontSize": 18, "color": "#1455AA", "opacity": 1 }
                ] },
                { "sourceIndex": null, "width": 300, "height": 200, "rotation": 90, "crop": null, "objects": [] },
                { "sourceIndex": 0, "width": 300, "height": 200, "rotation": 0, "crop": null, "objects": [] }
            ] }
        });
        request.as_object_mut().unwrap().extend(sections.as_object().unwrap().clone());
        serde_json::from_value(request).unwrap()
    }

    #[test]
    fn export_without_stage_sections_matches_the_overlay_only_composition() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); fixture(&input);
        let original=fs::read(&input).unwrap();
        let mut overlay_only=load(&input).unwrap();
        let request=wire_request(&input, serde_json::json!({}));
        let plan=overlay_preflight(&overlay_only,&request.scene).unwrap();
        stage_apply(plan,&mut overlay_only,&request.scene).unwrap();
        let mut expected=Vec::new(); overlay_only.save_to(&mut expected).unwrap();

        for sections in [serde_json::json!({}), serde_json::json!({ "formValues": null, "textEdits": null, "flatten": null, "redactions": null })] {
            let request=wire_request(&input, sections);
            assert!(request.sections.form_values.is_none() && request.sections.text_edits.is_none() && request.sections.flatten.is_none() && request.sections.redactions.is_none());
            let outcome=export(&request,input.clone());
            assert!(outcome.error().is_none(),"{:?}",outcome.error());
            assert_eq!(fs::read(&outcome.output_paths()[0]).unwrap(),expected);
            fs::remove_file(&outcome.output_paths()[0]).unwrap();
        }
        assert_eq!(fs::read(&input).unwrap(),original);
    }

    #[test]
    fn present_but_unimplemented_sections_are_rejected_before_output() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); fixture(&input);
        let original=fs::read(&input).unwrap();
        let cases=[
            (serde_json::json!({ "flatten": false }), "Flattening is not supported yet"),
        ];
        for (sections,message) in cases {
            let outcome=export(&wire_request(&input, sections),input.clone());
            assert!(outcome.error().is_some_and(|error| error.message.starts_with(message)),"{outcome:?}");
            assert!(outcome.output_paths().is_empty());
            assert_eq!(fs::read_dir(&temp.0).unwrap().count(),1);
            assert_eq!(fs::read(&input).unwrap(),original);
        }
    }

    #[test]
    fn stages_run_in_a_fixed_order_with_every_preflight_before_any_apply() {
        assert_eq!(STAGES,[Stage::FillForms,Stage::EditText,Stage::Overlays,Stage::Flatten,Stage::Redact]);
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); fixture(&input);
        let first_failure=|sections: serde_json::Value, scene: Option<PdfScene>| {
            let mut request=wire_request(&input, sections);
            if let Some(scene)=scene { request.scene=scene; }
            run_stages(&input,&request.scene,&request.sections).err().unwrap().message
        };
        let all=serde_json::json!({ "formValues": [], "textEdits": [], "flatten": true, "redactions": [] });
        assert!(first_failure(all, None).starts_with("This PDF has no form fields to fill"));
        assert!(first_failure(serde_json::json!({ "textEdits": [], "flatten": true, "redactions": [] }), None).starts_with("No text was changed"));
        assert!(first_failure(serde_json::json!({ "flatten": true, "redactions": [] }), Some(PdfScene{pages:vec![]})).starts_with("Scene must contain"));
        assert!(first_failure(serde_json::json!({ "flatten": true, "redactions": [] }), None).starts_with("Flattening"));
        assert!(first_failure(serde_json::json!({ "redactions": [] }), None).starts_with("Redaction"));
    }

    /// Three pages: page 1 holds text, a vector, an image, a hidden layer, a comment and a filled
    /// form field; page 3 holds a file attachment and a link. The catalog carries metadata, XMP,
    /// an embedded file and structure text.
    fn redaction_fixture(path: &Path) {
        let mut d = Document::with_version("1.7");
        let root = d.new_object_id();
        let font = d.add_object(dictionary! {"Type"=>"Font", "Subtype"=>"Type1", "BaseFont"=>"Helvetica"});
        let hidden = d.add_object(dictionary! {"Type"=>"OCG", "Name"=>Object::string_literal("Hidden layer")});
        let photo = d.add_object(Stream::new(dictionary! {"Type"=>"XObject","Subtype"=>"Image","Width"=>2,"Height"=>1,"ColorSpace"=>"DeviceGray","BitsPerComponent"=>8,"Name"=>"PhotoSecret"}, vec![0, 255]));
        let first_resources = d.add_object(dictionary! {"Font"=>dictionary!{"F1"=>font}, "XObject"=>dictionary!{"Photo"=>photo}, "Properties"=>dictionary!{"Hidden"=>hidden}});
        let resources = d.add_object(dictionary! {"Font"=>dictionary!{"F1"=>font}});
        let first_content = d.add_object(Stream::new(dictionary!{}, b"q 0.5 0.5 0.5 rg 20 600 380 180 re f Q BT /F1 18 Tf 40 700 Td (First page text) Tj ET q 50 0 0 50 400 400 cm /Photo Do Q /OC /Hidden BDC BT /F1 18 Tf 40 300 Td (Hidden layer text) Tj ET EMC".to_vec()));
        let second_content = d.add_object(Stream::new(dictionary!{}, b"BT /F1 18 Tf 40 700 Td (Second page text) Tj ET".to_vec()));
        let third_content = d.add_object(Stream::new(dictionary!{}, b"BT /F1 18 Tf 40 700 Td (Third page text) Tj ET".to_vec()));
        let first = d.new_object_id();
        let third = d.new_object_id();
        let comment = d.add_object(dictionary! {"Type"=>"Annot","Subtype"=>"Text","Rect"=>array(&[300.,300.,320.,320.]),"Contents"=>Object::string_literal("Secret note"),"P"=>first});
        let appearance = d.add_object(Stream::new(dictionary!{"Type"=>"XObject","Subtype"=>"Form","BBox"=>array(&[0.,0.,100.,20.]),"Resources"=>dictionary!{"Font"=>dictionary!{"F1"=>font}}}, b"BT /F1 12 Tf 2 5 Td (Secret value) Tj ET".to_vec()));
        let widget = d.add_object(dictionary! {"Type"=>"Annot","Subtype"=>"Widget","FT"=>"Tx","T"=>Object::string_literal("account"),"V"=>Object::string_literal("Secret value"),"Rect"=>array(&[40.,100.,140.,120.]),"AP"=>dictionary!{"N"=>appearance},"P"=>first});
        let attached = d.add_object(Stream::new(dictionary!{"Type"=>"EmbeddedFile"}, b"Attached secret".to_vec()));
        let file_spec = d.add_object(dictionary! {"Type"=>"Filespec","F"=>Object::string_literal("secret.txt"),"EF"=>dictionary!{"F"=>attached}});
        let attachment = d.add_object(dictionary! {"Type"=>"Annot","Subtype"=>"FileAttachment","Rect"=>array(&[10.,10.,30.,30.]),"FS"=>file_spec,"P"=>third});
        let link = d.add_object(dictionary! {"Type"=>"Annot","Subtype"=>"Link","Rect"=>array(&[100.,40.,160.,80.]),"A"=>dictionary!{"S"=>"URI","URI"=>"https://example.invalid"}});
        d.objects.insert(first, Object::Dictionary(dictionary! {"Type"=>"Page","Parent"=>root,"MediaBox"=>array(&[0.,0.,612.,792.]),"Resources"=>first_resources,"Contents"=>first_content,"Annots"=>vec![Object::Reference(comment),Object::Reference(widget)],"StructParents"=>0}));
        let second = d.add_object(dictionary! {"Type"=>"Page","Parent"=>root,"MediaBox"=>array(&[0.,0.,612.,792.]),"Resources"=>resources,"Contents"=>second_content});
        d.objects.insert(third, Object::Dictionary(dictionary! {"Type"=>"Page","Parent"=>root,"MediaBox"=>array(&[0.,0.,612.,792.]),"Resources"=>resources,"Contents"=>third_content,"Annots"=>vec![Object::Reference(attachment),Object::Reference(link)]}));
        d.objects.insert(root, Object::Dictionary(dictionary! {"Type"=>"Pages","Kids"=>vec![Object::Reference(first),Object::Reference(second),Object::Reference(third)],"Count"=>3}));
        let xmp = d.add_object(Stream::new(dictionary!{"Type"=>"Metadata","Subtype"=>"XML"}, b"<x:xmpmeta>XMP secret</x:xmpmeta>".to_vec()));
        let structure = d.add_object(dictionary! {"Type"=>"StructTreeRoot","K"=>dictionary!{"S"=>"P","ActualText"=>Object::string_literal("Structure secret"),"Pg"=>first}});
        let form = d.add_object(dictionary! {"Fields"=>vec![Object::Reference(widget)]});
        let catalog = d.add_object(dictionary! {"Type"=>"Catalog","Pages"=>root,"Metadata"=>xmp,"StructTreeRoot"=>structure,"MarkInfo"=>dictionary!{"Marked"=>true},"AcroForm"=>form,
            "Names"=>dictionary!{"EmbeddedFiles"=>dictionary!{"Names"=>vec![Object::string_literal("secret.txt"),Object::Reference(file_spec)]}},
            "OCProperties"=>dictionary!{"OCGs"=>vec![Object::Reference(hidden)],"D"=>dictionary!{"OFF"=>vec![Object::Reference(hidden)]}}});
        let info = d.add_object(dictionary! {"Author"=>Object::string_literal("Author secret")});
        d.trailer.set("Root", catalog);
        d.trailer.set("Info", info);
        d.save(path).unwrap();
    }

    fn identity_request(input: &Path, pages: usize, redactions: serde_json::Value) -> ExportRequest {
        serde_json::from_value(serde_json::json!({
            "paths": [input], "outputLocation": "alongsideInput",
            "scene": { "pages": (0..pages).map(|index| serde_json::json!({ "sourceIndex": index, "width": 612, "height": 792, "rotation": 0, "crop": null, "objects": [] })).collect::<Vec<_>>() },
            "redactions": redactions,
        })).unwrap()
    }

    fn redacted_image(document: &Document, page: ObjectId) -> (u32, u32, Vec<u8>) {
        let dict = document.get_dictionary(page).unwrap();
        let resources = dict.get(b"Resources").unwrap().as_dict().unwrap();
        assert_eq!(resources.len(), 1, "a redacted page keeps only its image resource");
        let xobjects = resources.get(b"XObject").unwrap().as_dict().unwrap();
        assert_eq!(xobjects.len(), 1);
        let image = document.get_object(xobjects.get(b"Redacted").unwrap().as_reference().unwrap()).unwrap().as_stream().unwrap();
        assert_eq!(image.dict.get(b"Subtype").unwrap().as_name().unwrap(), b"Image");
        assert_eq!(image.dict.get(b"Filter").unwrap().as_name().unwrap(), b"FlateDecode");
        let width = image.dict.get(b"Width").unwrap().as_i64().unwrap() as u32;
        let height = image.dict.get(b"Height").unwrap().as_i64().unwrap() as u32;
        (width, height, image.decompressed_content().unwrap())
    }

    fn pixel(image: &(u32, u32, Vec<u8>), x: f32, y: f32, page: (f32, f32)) -> [u8; 3] {
        let (px, py) = ((x * image.0 as f32 / page.0) as usize, (y * image.1 as f32 / page.1) as usize);
        let offset = (py * image.0 as usize + px) * 3;
        [image.2[offset], image.2[offset + 1], image.2[offset + 2]]
    }

    fn require_renderer() -> bool {
        match renderer() { Ok(_) => true, Err(e) => { if std::env::var_os("TOOLBOX_REQUIRE_PDF_RENDERER").is_some() { panic!("{e}") }; eprintln!("Renderer unavailable: {e}"); false } }
    }

    #[test]
    fn redaction_rasterises_only_marked_pages_and_removes_everything_under_them() {
        let _guard = crate::kit::lock_process_env();
        if !require_renderer() { return; }
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); redaction_fixture(&input);
        let original=fs::read(&input).unwrap();
        let source=load(&input).unwrap();
        let source_pages:Vec<ObjectId>=source.get_pages().values().copied().collect();
        let request=identity_request(&input,3,serde_json::json!([
            { "pageIndex": 0, "rect": { "x": 30, "y": 60, "width": 300, "height": 50 }, "color": "#000000" },
            { "pageIndex": 0, "rect": { "x": 380, "y": 330, "width": 100, "height": 100 }, "color": "#000000" },
        ]));
        let outcome=export(&request,input.clone());
        assert!(outcome.error().is_none(),"{:?}",outcome.error());
        let output=outcome.output_paths()[0].clone();
        assert_eq!(output.file_name().unwrap(),"source-redacted.pdf");
        assert!(outcome.detail.ends_with("Page 1 was redacted as a 300 dpi image. Document metadata, XMP data and attachments were removed."),"{:?}",outcome.detail);
        assert_eq!(fs::read(&input).unwrap(),original);

        let bytes=fs::read(&output).unwrap();
        for secret in ["First page text","Hidden layer text","Secret note","Secret value","XMP secret","Attached secret","Author secret","Structure secret","PhotoSecret"] {
            assert!(!bytes.windows(secret.len()).any(|window|window==secret.as_bytes()),"{secret} survived redaction");
        }
        let document=load(&output).unwrap();
        let pages:Vec<ObjectId>=document.get_pages().values().copied().collect();
        assert_eq!(pages,source_pages);

        let first=document.get_dictionary(pages[0]).unwrap();
        let mut keys:Vec<_>=first.iter().map(|(key,_)|String::from_utf8_lossy(key).into_owned()).collect(); keys.sort();
        assert_eq!(keys,["Contents","CropBox","MediaBox","Parent","Resources","Rotate","Type"],"annotations, widgets and structure keys are gone");
        let image=redacted_image(&document,pages[0]);
        assert_eq!((image.0,image.1),(2550,3300),"612×792 pt at 300 dpi");
        assert_eq!(pixel(&image,40.,80.,(612.,792.)),[0,0,0],"marked text area is filled");
        assert_eq!(pixel(&image,420.,370.,(612.,792.)),[0,0,0],"marked image area is filled");
        assert!(pixel(&image,360.,30.,(612.,792.)).iter().all(|value|(120..=135).contains(value)),"unmarked vector content stays as pixels");
        assert_eq!(pixel(&image,500.,700.,(612.,792.)),[255,255,255]);

        for index in [1,2] {
            let expected=source.get_object(source_pages[index]).unwrap();
            let actual=document.get_object(pages[index]).unwrap();
            if index==1 { assert_eq!(format!("{actual:?}"),format!("{expected:?}"),"untouched page is unchanged"); }
            let content=|doc:&Document,page:ObjectId| doc.get_object(doc.get_dictionary(page).unwrap().get(b"Contents").unwrap().as_reference().unwrap()).unwrap().as_stream().unwrap().content.clone();
            assert_eq!(content(&document,pages[index]),content(&source,source_pages[index]));
        }
        let third=document.get_dictionary(pages[2]).unwrap().get(b"Annots").unwrap().as_array().unwrap();
        assert_eq!(third.len(),1,"file attachments are removed from untouched pages");
        assert_eq!(document.get_dictionary(third[0].as_reference().unwrap()).unwrap().get(b"Subtype").unwrap().as_name().unwrap(),b"Link");

        let catalog=document.catalog().unwrap();
        for key in [&b"Metadata"[..],b"StructTreeRoot",b"MarkInfo",b"AcroForm",b"Names"] { assert!(!catalog.has(key),"{} survived",String::from_utf8_lossy(key)); }
        assert!(!document.trailer.has(b"Info"));
        assert!(document.objects.values().all(|object| object.as_stream().map_or(true,|stream| stream.dict.get(b"Type").ok().and_then(|t|t.as_name().ok())!=Some(b"EmbeddedFile"))));

        if let Some(extractor)=text_extractor() {
            let text=|page:usize| String::from_utf8(Command::new(&extractor).args(["-f",&page.to_string(),"-l",&page.to_string()]).arg(&output).arg("-").output().unwrap().stdout).unwrap();
            assert!(text(1).trim().is_empty(),"redacted page has no extractable text");
            assert!(text(2).contains("Second page text"));
        }

        let again=export(&request,input.clone());
        assert_eq!(again.output_paths()[0].file_name().unwrap(),"source-redacted-1.pdf","never overwrites an earlier output");
        assert_eq!(fs::read(&again.output_paths()[0]).unwrap(),bytes,"repeated runs are byte-identical");
    }

    #[test]
    fn redaction_fill_colour_renders() {
        let _guard = crate::kit::lock_process_env();
        if !require_renderer() { return; }
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); redaction_fixture(&input);
        let request=identity_request(&input,3,serde_json::json!([{ "pageIndex": 0, "rect": { "x": 30, "y": 60, "width": 300, "height": 50 }, "color": "#FFFFFF" }]));
        let outcome=export(&request,input.clone());
        assert!(outcome.error().is_none(),"{:?}",outcome.error());
        let document=load(&outcome.output_paths()[0]).unwrap();
        let image=redacted_image(&document,document.get_pages()[&1]);
        assert_eq!(pixel(&image,40.,80.,(612.,792.)),[255,255,255],"white fill replaces the grey vector under the mark");
        assert!(pixel(&image,360.,30.,(612.,792.)).iter().all(|value|(120..=135).contains(value)));
    }

    #[test]
    fn redaction_follows_the_visible_page_of_rotated_and_cropped_sources() {
        let _guard = crate::kit::lock_process_env();
        if !require_renderer() { return; }
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); fixture(&input);
        let request:ExportRequest=serde_json::from_value(serde_json::json!({
            "paths": [input], "outputLocation": "alongsideInput",
            "scene": { "pages": [
                { "sourceIndex": 0, "width": 300, "height": 200, "rotation": 0, "crop": null, "objects": [] },
                { "sourceIndex": 1, "width": 612, "height": 792, "rotation": 0, "crop": null, "objects": [] }
            ] },
            "redactions": [{ "pageIndex": 0, "rect": { "x": 0, "y": 0, "width": 150, "height": 100 }, "color": "#000000" }],
        })).unwrap();
        let outcome=export(&request,input.clone());
        assert!(outcome.error().is_none(),"{:?}",outcome.error());
        let document=load(&outcome.output_paths()[0]).unwrap();
        let page=document.get_pages()[&1];
        let dict=document.get_dictionary(page).unwrap();
        assert_eq!(bounds(&document,dict.get(b"MediaBox").unwrap()).unwrap(),[0.,0.,300.,200.]);
        assert_eq!(dict.get(b"Rotate").unwrap().as_i64().unwrap(),0);
        let image=redacted_image(&document,page);
        assert_eq!((image.0,image.1),(1250,834));
        assert_eq!(pixel(&image,10.,10.,(300.,200.)),[0,0,0]);
        assert_eq!(pixel(&image,140.,90.,(300.,200.)),[0,0,0]);
        assert_ne!(pixel(&image,160.,110.,(300.,200.)),[0,0,0]);
    }

    #[test]
    fn redaction_marks_follow_the_canvas_rotation() {
        let page:ScenePage=serde_json::from_value(serde_json::json!({ "sourceIndex": 0, "width": 300, "height": 200, "rotation": 90, "crop": null, "objects": [] })).unwrap();
        let (rect,width,height)=displayed(&page,&Rect{x:0.,y:0.,width:150.,height:100.});
        assert_eq!((width,height),(200.,300.));
        assert_eq!((rect.x,rect.y,rect.width,rect.height),(100.,0.,100.,150.));
        let cropped:ScenePage=serde_json::from_value(serde_json::json!({ "sourceIndex": 0, "width": 300, "height": 200, "rotation": 0, "crop": { "x": 50, "y": 40, "width": 100, "height": 100 }, "objects": [] })).unwrap();
        let (rect,width,height)=displayed(&cropped,&Rect{x:60.,y:50.,width:10.,height:10.});
        assert_eq!((width,height,rect.x,rect.y),(100.,100.,10.,10.));
    }

    #[test]
    fn redaction_with_other_edits_uses_the_combined_suffix() {
        let _guard = crate::kit::lock_process_env();
        if !require_renderer() { return; }
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); fixture(&input);
        let outcome=export(&wire_request(&input, serde_json::json!({ "redactions": [
            { "pageIndex": 0, "rect": { "x": 60, "y": 140, "width": 400, "height": 60 }, "color": "#000000" },
            { "pageIndex": 2, "rect": { "x": 0, "y": 0, "width": 10, "height": 10 }, "color": "#000000" },
        ] })),input.clone());
        assert!(outcome.error().is_none(),"{:?}",outcome.error());
        assert_eq!(outcome.output_paths()[0].file_name().unwrap(),"source-edited.pdf");
        assert!(outcome.detail.ends_with("Pages 1 and 3 were redacted as 300 dpi images. Document metadata, XMP data and attachments were removed."),"{:?}",outcome.detail);
        let bytes=fs::read(&outcome.output_paths()[0]).unwrap();
        assert!(!bytes.windows(6).any(|window|window==b"Staged"),"overlays are redacted too because redaction runs last");
    }

    #[test]
    fn invalid_redactions_are_rejected_before_output() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); page_text_fixture(&input);
        let original=fs::read(&input).unwrap();
        let mark=|page:usize,x:f32,color:&str| serde_json::json!({ "pageIndex": page, "rect": { "x": x, "y": 10, "width": 20, "height": 20 }, "color": color });
        for (marks,message) in [
            (serde_json::json!([]),"Redaction needs between 1 and"),
            (serde_json::json!([mark(2,10.,"#000000")]),"A redaction mark is on a page that is not in the document"),
            (serde_json::json!([mark(0,700.,"#000000")]),"A redaction mark is outside page 1"),
            (serde_json::json!([mark(0,10.,"black")]),"Color must be #RRGGBB"),
        ] {
            let outcome=export(&identity_request(&input,2,marks),input.clone());
            assert!(outcome.error().is_some_and(|error| error.message.starts_with(message)),"{outcome:?}");
            assert_eq!(fs::read_dir(&temp.0).unwrap().count(),1);
            assert_eq!(fs::read(&input).unwrap(),original);
        }
    }

    fn text_object(text: &str, y: f32) -> SceneObject {
        SceneObject { rect: Rect{x:40.,y,width:400.,height:60.}, text: text.into(), font_size: 20., color: "#202020".into(), opacity: 1., ..object(Kind::Text) }
    }
    fn text_scene(objects: Vec<SceneObject>) -> PdfScene {
        PdfScene { pages: vec![
            ScenePage { source_index:Some(0), width:612., height:792., rotation:0, crop:None, source_rotation:None, source_box:None, objects },
            ScenePage { source_index:Some(1), width:612., height:792., rotation:0, crop:None, source_rotation:None, source_box:None, objects:vec![] },
            ScenePage { source_index:None, width:612., height:792., rotation:0, crop:None, source_rotation:None, source_box:None, objects:vec![] },
        ] }
    }
    const SCRIPTS: [&str; 3] = ["Hello, world", "Привет, мир", "Καλημέρα κόσμε"];
    fn script_objects() -> Vec<SceneObject> {
        let mut serif = text_object(SCRIPTS[1], 220.); serif.font_family = Some("Noto Serif".into()); serif.bold = true;
        let mut mono = text_object(SCRIPTS[2], 340.); mono.font_family = Some("Noto Sans Mono".into()); mono.italic = true;
        vec![text_object(SCRIPTS[0], 100.), serif, mono]
    }
    fn export_text(input: &Path, scene: PdfScene) -> JobOutcome {
        export(&ExportRequest { paths: vec![input.to_path_buf()], scene, output_location: OutputLocation::AlongsideInput, output_path: None, sections: Default::default() }, input.to_path_buf())
    }

    #[test]
    fn text_overlays_embed_subset_bundled_fonts_and_export_byte_identically() {
        let temp = TempDir::new().unwrap(); let input = temp.0.join("source.pdf"); page_text_fixture(&input);
        let first = export_text(&input, text_scene(script_objects()));
        let second = export_text(&input, text_scene(script_objects()));
        assert!(first.error().is_none() && second.error().is_none(), "{:?} {:?}", first.error(), second.error());
        assert_ne!(first.output_paths()[0], second.output_paths()[0]);
        assert_eq!(fs::read(&first.output_paths()[0]).unwrap(), fs::read(&second.output_paths()[0]).unwrap());

        let doc = Document::load(&first.output_paths()[0]).unwrap();
        let mut embedded = Vec::new();
        for object in doc.objects.values() {
            let Ok(font) = object.as_dict() else { continue };
            if font.get(b"Subtype").and_then(Object::as_name).ok() != Some(b"Type0".as_slice()) { continue; }
            assert_eq!(font.get(b"Encoding").unwrap().as_name().unwrap(), b"Identity-H");
            assert!(font.has(b"ToUnicode"));
            let base = String::from_utf8(font.get(b"BaseFont").unwrap().as_name().unwrap().to_vec()).unwrap();
            let (tag, name) = base.split_once('+').unwrap();
            assert!(tag.len() == 6 && tag.chars().all(|c| c.is_ascii_uppercase()), "{base}");
            let descendant = doc.get_dictionary(font.get(b"DescendantFonts").unwrap().as_array().unwrap()[0].as_reference().unwrap()).unwrap();
            assert_eq!(descendant.get(b"Subtype").unwrap().as_name().unwrap(), b"CIDFontType2");
            let descriptor = doc.get_dictionary(descendant.get(b"FontDescriptor").unwrap().as_reference().unwrap()).unwrap();
            let file = doc.get_object(descriptor.get(b"FontFile2").unwrap().as_reference().unwrap()).unwrap().as_stream().unwrap();
            let subset = file.decompressed_content().unwrap();
            let full = match name { "NotoSans-Regular" => NOTO_SANS_REGULAR, "NotoSerif-Bold" => NOTO_SERIF_BOLD, "NotoSansMono-Regular" => NOTO_SANS_MONO_REGULAR, other => panic!("unexpected font {other}") };
            assert!(subset.len() * 10 < full.len(), "{name} was not subset: {} of {} bytes", subset.len(), full.len());
            embedded.push(name.to_string());
        }
        embedded.sort();
        assert_eq!(embedded, ["NotoSans-Regular", "NotoSansMono-Regular", "NotoSerif-Bold"]);
        let page = doc.get_pages()[&1];
        let content = String::from_utf8_lossy(&doc.get_page_content(page)).into_owned();
        assert!(content.contains("1 0 0.2 -1 "), "mono italic must be slanted: {content}");
    }

    #[test]
    fn latin_cyrillic_and_greek_overlay_text_extracts_exactly() {
        let Some(extractor) = text_extractor() else {
            if std::env::var_os("TOOLBOX_REQUIRE_PDF_RENDERER").is_some() { panic!("pdftotext is unavailable") }
            eprintln!("pdftotext unavailable"); return;
        };
        let temp = TempDir::new().unwrap(); let input = temp.0.join("source.pdf"); page_text_fixture(&input);
        let outcome = export_text(&input, text_scene(script_objects()));
        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        let output = Command::new(extractor).args(["-enc", "UTF-8", "-f", "1", "-l", "1"]).arg(&outcome.output_paths()[0]).arg("-").output().unwrap();
        let text = String::from_utf8(output.stdout).unwrap();
        let lines: Vec<&str> = text.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
        for script in SCRIPTS { assert!(lines.contains(&script), "{script:?} not extracted from {lines:?}"); }
    }

    #[test]
    fn text_overlay_preview_and_export_are_pixel_identical() {
        let _guard = crate::kit::lock_process_env();
        let renderer=match renderer() { Ok(r)=>r,Err(e)=>{if std::env::var_os("TOOLBOX_REQUIRE_PDF_RENDERER").is_some(){panic!("{e}")}; eprintln!("Renderer unavailable: {e}");return;} };
        let temp = TempDir::new().unwrap(); let input = temp.0.join("source.pdf"); page_text_fixture(&input);
        let mut objects = script_objects();
        let mut wrapped = text_object("Centred text that wraps inside its box\nand a second paragraph", 450.);
        wrapped.rect.width = 160.; wrapped.rect.height = 140.; wrapped.align = TextAlign::Center; wrapped.page_range = Some("2-".into());
        objects.push(wrapped);
        let outcome = export_text(&input, text_scene(objects.clone()));
        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        for index in 0..3 {
            let preview = preview(&PreviewRequest { path: input.clone(), scene: text_scene(objects.clone()), page_index: index, sections: Default::default() }).unwrap();
            let actual = render(&outcome.output_paths()[0], index, &renderer, &temp).unwrap();
            assert_eq!(preview.data_url, actual.data_url, "page {index} differs between preview and export");
        }
        let blank = preview(&PreviewRequest { path: input.clone(), scene: text_scene(vec![]), page_index: 2, sections: Default::default() }).unwrap();
        let ranged = preview(&PreviewRequest { path: input.clone(), scene: text_scene(objects), page_index: 2, sections: Default::default() }).unwrap();
        assert_ne!(blank.data_url, ranged.data_url, "ranged text must draw on page 3");
    }

    #[test]
    fn over_limit_text_returns_limit_exceeded_without_output() {
        let temp = TempDir::new().unwrap(); let input = temp.0.join("source.pdf"); page_text_fixture(&input);
        assert!(layout_text(&text_object(&"a".repeat(100_000), 100.)).is_ok());
        let outcome = export_text(&input, text_scene(vec![text_object(&"a".repeat(100_001), 100.)]));
        let error = outcome.error().unwrap();
        assert_eq!(error.kind, crate::kit::contracts::ErrorKind::LimitExceeded { limit: crate::kit::contracts::LimitKind::TextLength });
        assert!(error.message.contains("100,000"), "{}", error.message);
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 1);
    }

    #[test]
    fn unrenderable_characters_are_named_and_nothing_is_written() {
        let temp = TempDir::new().unwrap(); let input = temp.0.join("source.pdf"); page_text_fixture(&input);
        let scene = || text_scene(vec![text_object("Hi \u{1f600} आशा \u{1f600}", 100.)]);
        let outcome = export_text(&input, scene());
        let error = outcome.error().unwrap();
        assert_eq!(error.kind, crate::kit::contracts::ErrorKind::InvalidInput);
        assert_eq!(error.message, "Noto Sans can't show these characters: “\u{1f600}” (U+1F600), “आ” (U+0906), “श” (U+0936), “ा” (U+093E). Remove them to export this text box.");
        assert_eq!(compose(&input, &scene()).err().unwrap(), error.message);
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 1);
        let many: String = ('Ա'..='Ք').collect();
        let message = layout_text(&text_object(&many, 100.)).err().unwrap().message;
        assert!(message.contains("(U+0531)") && message.ends_with("“Ժ” (U+053A) and 26 more. Remove them to export this text box."), "{message}");
    }

    #[test]
    fn page_ranges_expand_and_clamp_to_the_document() {
        assert_eq!(page_range_pages("2-", 3).unwrap(), vec![1, 2]);
        assert_eq!(page_range_pages("1-99", 3).unwrap(), vec![0, 1, 2]);
        assert_eq!(page_range_pages(" 3, 1 ,1", 3).unwrap(), vec![0, 2]);
        assert_eq!(page_range_pages("all", 4).unwrap(), vec![0, 1, 2, 3]);
        assert_eq!(page_range_pages("Odd", 5).unwrap(), vec![0, 2, 4]);
        assert_eq!(page_range_pages("even", 5).unwrap(), vec![1, 3]);
        assert_eq!(page_range_pages("even, 1", 4).unwrap(), vec![0, 1, 3]);
        assert_eq!(page_range_pages("5-", 7).unwrap(), vec![4, 5, 6]);
        assert_eq!(page_range_pages("even", 1).unwrap_err().message, "Pages even are not in this document. It has 1 page.");
        for range in ["", "0", "3-1", "a", "1,,2", "-2", "1-2-3", "odd-2", "evens"] {
            assert_eq!(page_range_pages(range, 3).unwrap_err().kind, crate::kit::contracts::ErrorKind::InvalidInput, "{range:?}");
        }
        assert_eq!(page_range_pages("5-", 3).unwrap_err().message, "Pages 5- are not in this document. It has 3 pages.");

        let temp = TempDir::new().unwrap(); let input = temp.0.join("source.pdf"); page_text_fixture(&input);
        let mut ranged = text_object("Draft", 100.); ranged.page_range = Some("2-".into());
        let doc = compose(&input, &text_scene(vec![ranged])).unwrap();
        let pages: Vec<_> = doc.get_pages().values().copied().collect();
        let source = Document::load(&input).unwrap();
        assert_eq!(doc.get_page_content(pages[0]), source.get_page_content(source.get_pages()[&1]), "page 1 must stay untouched");
        for page in &pages[1..] { assert!(String::from_utf8_lossy(&doc.get_page_content(*page)).contains("/SceneTextNotoSansRegular 20 Tf")); }

        let mut highlight = object(Kind::Highlight); highlight.page_range = Some("1".into());
        assert_eq!(compose(&input, &text_scene(vec![highlight])).unwrap_err(), "Only text boxes, signatures and watermarks can be shown on several pages");
    }

    #[test]
    fn text_wraps_inside_the_box_and_aligns_each_line() {
        let face = parse_face(NOTO_SANS_REGULAR).unwrap();
        let width = |text: &str| text.chars().map(|c| glyph_width(&face, face.glyph_index(c).unwrap().0) as f32 * 0.02).sum::<f32>();
        let mut o = text_object("alpha beta gamma\n\nlongwordlongword", 100.);
        o.rect.width = width("alpha beta") + 1.;
        let layout = layout_text(&o).unwrap();
        let lines: Vec<String> = layout.lines.iter().map(|line| line.glyphs.iter().map(|(_, c)| *c).collect()).collect();
        assert_eq!(lines[..3], ["alpha beta", "gamma", ""]);
        assert!(lines[3..].iter().all(|line| width(line) <= o.rect.width), "{lines:?}");
        assert_eq!(lines[3..].concat(), "longwordlongword");
        let remapper = subsetter::GlyphRemapper::new();
        for (align, offset) in [(TextAlign::Left, 0.), (TextAlign::Center, (o.rect.width - width("gamma")) / 2.), (TextAlign::Right, o.rect.width - width("gamma"))] {
            o.align = align;
            let content = text_content(&o, &layout, "F", &remapper);
            let y = o.rect.y + layout.first_baseline + 20. * TEXT_LINE_HEIGHT;
            assert!(content.contains(&format!("1 0 0 -1 {} {y} Tm", o.rect.x + offset)), "{content}");
        }
    }

    #[test]
    fn text_fonts_follow_family_bold_and_italic() {
        let font = |family: Option<&str>, bold, italic| text_font(&SceneObject { font_family: family.map(Into::into), bold, italic, ..object(Kind::Text) });
        assert_eq!(font(None, false, false).unwrap().name, "NotoSans-Regular");
        assert_eq!(font(Some("Noto Sans"), true, true).unwrap().name, "NotoSans-BoldItalic");
        assert_eq!(font(Some("Noto Serif"), false, true).unwrap().name, "NotoSerif-Italic");
        let mono = font(Some("Noto Sans Mono"), true, true).unwrap();
        assert_eq!((mono.name, mono.oblique), ("NotoSansMono-Bold", true));
        assert!(!font(Some("Noto Sans"), false, true).unwrap().oblique);
        assert_eq!(font(Some("Helvetica"), false, false).err().unwrap().kind, crate::kit::contracts::ErrorKind::InvalidInput);
    }

    fn page_contents(doc: &Document) -> Vec<String> {
        doc.get_pages().values().map(|page| String::from_utf8_lossy(&doc.get_page_content(*page)).into_owned()).collect()
    }
    fn compose_watermark(input: &Path, object: SceneObject) -> Result<Document, String> { compose(input, &text_scene(vec![object])) }
    fn picture(path: &Path, width: u32, height: u32) {
        let mut image = image::RgbaImage::from_pixel(width, height, image::Rgba([200, 0, 0, 255]));
        image.put_pixel(0, 0, image::Rgba([0, 0, 0, 0]));
        image.save(path).unwrap();
    }

    #[test]
    fn watermark_positions_keep_each_mark_inside_the_visible_area() {
        let area = Rect { x: 10., y: 20., width: 600., height: 800. };
        let at = |placement, rotation| watermark_centers(&area, (100., 20.), Some(WatermarkPosition::Placement(placement)), rotation).unwrap();
        assert_eq!(at(WatermarkPlacement::Center, 0.), vec![(310., 420.)]);
        assert_eq!(at(WatermarkPlacement::TopLeft, 0.), vec![(84., 54.)]);
        assert_eq!(at(WatermarkPlacement::TopRight, 0.), vec![(536., 54.)]);
        assert_eq!(at(WatermarkPlacement::BottomLeft, 0.), vec![(84., 786.)]);
        assert_eq!(at(WatermarkPlacement::BottomRight, 0.), vec![(536., 786.)]);
        let [(x, y)] = at(WatermarkPlacement::TopLeft, 90.)[..] else { panic!() };
        assert!((x - 44.).abs() < 0.001 && (y - 94.).abs() < 0.001, "a quarter turn swaps the corner box: {x} {y}");
        assert_eq!(watermark_centers(&area, (100., 20.), Some(WatermarkPosition::Offset { x: 5, y: 7 }), 0.).unwrap(), vec![(65., 37.)]);
        assert_eq!(watermark_centers(&area, (100., 20.), None, 0.).unwrap(), vec![(310., 420.)]);

        let tiles = at(WatermarkPlacement::Tile, -35.);
        assert!(tiles.contains(&(310., 420.)));
        for corner in [(10., 20.), (610., 20.), (10., 820.), (610., 820.)] {
            assert!(tiles.iter().any(|(x, y)| (x - corner.0).abs() < 140. && (y - corner.1).abs() < 140.), "tiles must cover {corner:?}");
        }
        let error = watermark_centers(&area, (0.5, 0.5), Some(WatermarkPosition::Placement(WatermarkPlacement::Tile)), 0.).unwrap_err();
        assert_eq!(error.message, "The watermark is too small to tile across this page. Make it larger.");
    }

    #[test]
    fn legacy_patterns_map_to_clockwise_rotation_matrices() {
        let matrix = |rotation| watermark_rotation(rotation, (1., 2.));
        assert_eq!(matrix(0.), "1 0 0 1 1 2 cm\n");
        assert_eq!(matrix(90.), "0 1 -1 0 1 2 cm\n");
        // Scene space points down, so −45° rises from bottom left to top right.
        let rising: Vec<f32> = matrix(-45.).split(' ').take(4).map(|value| value.parse().unwrap()).collect();
        assert!(rising[0] > 0.7 && rising[1] < -0.7 && rising[2] > 0.7 && rising[3] > 0.7, "{rising:?}");
        let falling: Vec<f32> = matrix(45.).split(' ').take(4).map(|value| value.parse().unwrap()).collect();
        assert!(falling[1] > 0.7, "{falling:?}");
    }

    #[test]
    fn text_watermarks_use_every_shared_option() {
        let temp = TempDir::new().unwrap(); let input = temp.0.join("source.pdf"); page_text_fixture(&input);
        let options = WatermarkOptions { size: Some(0.2), rotation: -35., position: Some(WatermarkPosition::Placement(WatermarkPlacement::Tile)), ..text_watermark("DRAFT") };
        let mut object = watermark_object(options, None); object.font_family = Some("Noto Serif".into()); object.bold = true;
        let doc = compose_watermark(&input, object).unwrap();
        let content = &page_contents(&doc)[0];
        assert!(content.contains("0 0 1 rg"), "colour: {content}");
        assert!(content.contains(&format!("/SceneTextNotoSerifBold {} Tf", 0.2 * 612.)), "size is a fraction of the shorter side: {content}");
        assert!(content.matches(" Tj").count() > 4, "tile repeats the mark: {content}");
        let resources = doc.get_dictionary(doc.get_pages()[&1]).unwrap().get(b"Resources").unwrap().as_dict().unwrap();
        let states = resources.get(b"ExtGState").unwrap().as_dict().unwrap();
        assert!(states.iter().any(|(_, state)| state.as_dict().is_ok_and(|state| state.get(b"ca").and_then(Object::as_float).ok() == Some(0.3))), "opacity: {states:?}");
        assert!(content.contains("1 0 0 -1 "), "text must stay upright in the flipped page space: {content}");
    }

    #[test]
    fn watermarks_apply_to_all_odd_even_and_clamped_ranges_once() {
        let temp = TempDir::new().unwrap(); let input = temp.0.join("source.pdf"); page_text_fixture(&input);
        let source = Document::load(&input).unwrap();
        let source_second = source.get_page_content(source.get_pages()[&2]);
        for (range, expected) in [(None, [true, false, false]), (Some("all"), [true, true, true]), (Some("odd"), [true, false, true]),
            (Some("even"), [false, true, false]), (Some("2-9"), [false, true, true]), (Some("3-"), [false, false, true])] {
            let doc = compose_watermark(&input, watermark_object(text_watermark("DRAFT"), range)).unwrap();
            let contents = page_contents(&doc);
            for (index, marked) in expected.into_iter().enumerate() {
                assert_eq!(contents[index].contains("/SceneTextNotoSansRegular"), marked, "{range:?} page {}", index + 1);
            }
            if !expected[1] { assert_eq!(doc.get_page_content(doc.get_pages()[&2]), source_second, "{range:?} must leave page 2 untouched"); }
        }
        assert_eq!(compose_watermark(&input, watermark_object(text_watermark("DRAFT"), Some("5-"))).unwrap_err(), "Pages 5- are not in this document. It has 3 pages.");
    }

    #[test]
    fn watermark_layers_draw_behind_or_in_front_of_page_content() {
        let temp = TempDir::new().unwrap(); let input = temp.0.join("source.pdf"); page_text_fixture(&input);
        for (layer, behind) in [(Layer::Behind, true), (Layer::Front, false)] {
            let doc = compose_watermark(&input, SceneObject { layer, ..watermark_object(text_watermark("DRAFT"), Some("all")) }).unwrap();
            let content = &page_contents(&doc)[0];
            let (mark, original) = (content.find(" Tj").unwrap(), content.find("(First page text)").unwrap());
            assert_eq!(mark < original, behind, "{layer:?}: {content}");
        }
        let temp = TempDir::new().unwrap(); let cropped = temp.0.join("cropped.pdf"); fixture(&cropped);
        let mut model = scene();
        model.pages[0].objects = vec![SceneObject { layer: Layer::Behind, ..watermark_object(text_watermark("DRAFT"), None) }];
        let content = page_contents(&compose(&cropped, &model).unwrap())[0].clone();
        assert!(content.find(" Tj").unwrap() < content.find("/Original Do").unwrap(), "{content}");
    }

    #[test]
    fn picture_watermarks_embed_once_and_keep_transparency() {
        let temp = TempDir::new().unwrap(); let input = temp.0.join("source.pdf"); page_text_fixture(&input);
        let png = temp.0.join("logo.png"); picture(&png, 40, 20);
        let options = WatermarkOptions { text: None, logo_path: Some(png), color: None, size: Some(0.25), position: Some(WatermarkPosition::Placement(WatermarkPlacement::BottomRight)), ..text_watermark("") };
        let doc = compose_watermark(&input, watermark_object(options.clone(), Some("all"))).unwrap();
        let images: Vec<_> = doc.objects.iter().filter(|(_, value)| value.as_stream().is_ok_and(|stream| stream.dict.get(b"Subtype").ok().and_then(|v| v.as_name().ok()) == Some(b"Image") && stream.dict.has(b"SMask"))).map(|(id, _)| *id).collect();
        assert_eq!(images.len(), 1, "the picture is embedded once");
        let height = 0.25 * 612.;
        for (index, page) in doc.get_pages().values().enumerate() {
            let content = String::from_utf8_lossy(&doc.get_page_content(*page)).into_owned();
            assert!(content.contains(&format!("{} 0 0 {} {} {} cm /Wm0 Do", height * 2., -height, -height, height / 2.)), "page {index}: {content}");
            let resources = doc.get_dictionary(*page).unwrap().get(b"Resources").unwrap().as_dict().unwrap();
            assert_eq!(resources.get(b"XObject").unwrap().as_dict().unwrap().get(b"Wm0").unwrap().as_reference().unwrap(), images[0]);
        }

        let jpg = temp.0.join("logo.jpg");
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(30, 30, image::Rgba([0, 0, 200, 255]))).to_rgb8().save(&jpg).unwrap();
        let options = WatermarkOptions { text: None, logo_path: Some(jpg), color: None, ..text_watermark("") };
        let content = page_contents(&compose_watermark(&input, watermark_object(options, None)).unwrap())[0].clone();
        assert!(content.contains("30 0 0 -30 -15 15 cm /Wm0 Do"), "without a size a picture keeps its legacy size: {content}");

        let both = WatermarkOptions { text: Some("DRAFT".into()), color: None, ..options_with_logo(&temp) };
        assert_eq!(compose_watermark(&input, watermark_object(both, None)).unwrap_err(), "Choose text or a picture for the watermark, not both.");
        let none = SceneObject { watermark: None, ..object(Kind::Watermark) };
        assert_eq!(compose_watermark(&input, none).unwrap_err(), "Add watermark text or choose a picture.");
    }
    fn options_with_logo(temp: &TempDir) -> WatermarkOptions {
        let png = temp.0.join("small.png"); picture(&png, 4, 4);
        WatermarkOptions { text: None, logo_path: Some(png), color: None, ..text_watermark("") }
    }

    #[test]
    fn oversized_or_unsupported_watermark_pictures_are_refused_before_output() {
        let temp = TempDir::new().unwrap(); let input = temp.0.join("source.pdf"); page_text_fixture(&input);
        let huge = temp.0.join("huge.png"); fs::write(&huge, crate::kit::common::limits::fixtures::png_header(5000, 5000)).unwrap();
        let error = check_watermark_image(&huge).unwrap_err();
        assert_eq!(error.kind, crate::kit::contracts::ErrorKind::LimitExceeded { limit: LimitKind::Decompression });
        assert_eq!(error.message, "This picture is larger than 20 megapixels, the limit for a watermark. Use a smaller picture.");
        assert!(check_watermark_image(&{ let path = temp.0.join("edge.png"); fs::write(&path, crate::kit::common::limits::fixtures::png_header(5000, 4000)).unwrap(); path }).is_ok());

        let heavy = temp.0.join("heavy.png");
        fs::File::create(&heavy).unwrap().set_len(250 * 1024 * 1024 + 1).unwrap();
        let error = check_watermark_image(&heavy).unwrap_err();
        assert_eq!(error.kind, crate::kit::contracts::ErrorKind::LimitExceeded { limit: LimitKind::FileSize });
        assert_eq!(inspect_watermark_image(&heavy).unwrap_err(), error.message);
        fs::remove_file(&heavy).unwrap();

        let webp = temp.0.join("logo.webp");
        image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 0, 255])).save(&webp).unwrap();
        assert_eq!(check_watermark_image(&webp).unwrap_err().message, "Choose a PNG or JPG picture for the watermark.");

        let options = WatermarkOptions { text: None, logo_path: Some(huge), color: None, ..text_watermark("") };
        let before = fs::read_dir(&temp.0).unwrap().count();
        let outcome = export_text(&input, text_scene(vec![watermark_object(options, Some("all"))]));
        assert_eq!(outcome.error().unwrap().kind, crate::kit::contracts::ErrorKind::LimitExceeded { limit: LimitKind::Decompression });
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(), before, "nothing is written");
        let small = check_watermark_image(&options_with_logo(&temp).logo_path.unwrap()).unwrap();
        assert_eq!((small.width, small.height), (4, 4));
    }

    #[test]
    fn watermark_exports_are_byte_identical_across_runs() {
        let temp = TempDir::new().unwrap(); let input = temp.0.join("source.pdf"); page_text_fixture(&input);
        let objects = || vec![
            watermark_object(WatermarkOptions { rotation: -35., position: Some(WatermarkPosition::Placement(WatermarkPlacement::Tile)), ..text_watermark("CONFIDENTIAL") }, Some("odd")),
            SceneObject { layer: Layer::Behind, ..watermark_object(options_with_logo(&temp), Some("all")) },
        ];
        let first = export_text(&input, text_scene(objects()));
        let second = export_text(&input, text_scene(objects()));
        assert!(first.error().is_none() && second.error().is_none(), "{:?} {:?}", first.error(), second.error());
        assert_ne!(first.output_paths()[0], second.output_paths()[0]);
        assert_eq!(fs::read(&first.output_paths()[0]).unwrap(), fs::read(&second.output_paths()[0]).unwrap());
        assert_eq!(fs::read(&input).unwrap(), { let mut fresh = temp.0.join("fresh.pdf"); page_text_fixture(&fresh); let bytes = fs::read(&fresh).unwrap(); fresh.pop(); bytes }, "the original is not modified");
    }

    fn text_edit_fixture(path: &Path, embedded: bool) {
        let mut d = Document::with_version("1.7");
        let root = d.new_object_id();
        let mut helvetica = dictionary! {"Type"=>"Font", "Subtype"=>"Type1", "BaseFont"=>"Helvetica", "Encoding"=>"WinAnsiEncoding"};
        if embedded {
            let file = d.add_object(Stream::new(dictionary!{}, b"glyphs".to_vec()));
            let descriptor = d.add_object(dictionary! {"Type"=>"FontDescriptor", "FontName"=>"ABCDEF+Helvetica", "Flags"=>32, "FontFile2"=>file});
            helvetica = dictionary! {"Type"=>"Font", "Subtype"=>"TrueType", "BaseFont"=>"ABCDEF+Helvetica", "Encoding"=>"WinAnsiEncoding", "FontDescriptor"=>descriptor};
        }
        let font = d.add_object(helvetica);
        let cid_font = d.add_object(dictionary! {"Type"=>"Font", "Subtype"=>"Type0", "BaseFont"=>"Identity", "Encoding"=>"Identity-H"});
        let image = d.add_object(Stream::new(dictionary! {"Type"=>"XObject", "Subtype"=>"Image", "Width"=>1, "Height"=>1, "ColorSpace"=>"DeviceGray", "BitsPerComponent"=>8}, vec![0]));
        let resources = d.add_object(dictionary! {"Font"=>dictionary! {"OriginalFont"=>font, "CidFont"=>cid_font}, "XObject"=>dictionary! {"Scan"=>image}});
        let content = d.add_object(Stream::new(dictionary!{}, b"BT /OriginalFont 18 Tf 1 0 0 rg 40 700 Td (Original content) Tj ET \
            q 100 0 0 50 40 600 cm /Scan Do Q BT /CidFont 12 Tf 40 500 Td <0001> Tj ET \
            /Span <</MCID 0>> BDC BT /OriginalFont 12 Tf 40 400 Td (Tagged words) Tj ET EMC".to_vec()));
        let kids: Vec<Object> = (0..2).map(|_| Object::Reference(d.add_object(dictionary! {"Type"=>"Page", "Parent"=>root, "MediaBox"=>array(&[0.,0.,612.,792.]), "Resources"=>resources, "Contents"=>content}))).collect();
        d.objects.insert(root, Object::Dictionary(dictionary! {"Type"=>"Pages", "Kids"=>kids, "Count"=>2}));
        let catalog = d.add_object(dictionary! {"Type"=>"Catalog","Pages"=>root});
        d.trailer.set("Root",catalog);
        d.save(path).unwrap();
    }

    fn edit_request(input: &Path, edits: serde_json::Value) -> ExportRequest {
        let page = serde_json::json!({ "width": 612, "height": 792, "rotation": 0, "crop": null, "objects": [] });
        let pages: Vec<_> = (0..2).map(|index| { let mut page = page.clone(); page["sourceIndex"] = index.into(); page }).collect();
        serde_json::from_value(serde_json::json!({ "paths": [input], "outputLocation": "alongsideInput", "scene": { "pages": pages }, "textEdits": edits })).unwrap()
    }

    fn page_runs(path: &Path, page_index: usize) -> Vec<text_edit::TextEditRun> {
        let doc = load(path).unwrap();
        let id = doc.get_pages().values().copied().nth(page_index).unwrap();
        let matrix = geometry(&doc, id).unwrap().matrix;
        text_edit::page_text(&doc, id, matrix).unwrap().runs()
    }

    #[test]
    fn page_runs_mark_editable_text_and_explain_the_rest() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); text_edit_fixture(&input, false);
        let runs=page_runs(&input,1);
        let summary: Vec<_>=runs.iter().map(|run| (run.text.as_str(), run.reason)).collect();
        assert_eq!(summary,[("Original content",None),("",Some(text_edit::NotEditable::Picture)),("",Some(text_edit::NotEditable::Encoding)),("Tagged words",Some(text_edit::NotEditable::Tagged))]);
        let first=&runs[0];
        assert_eq!((first.font_name.as_str(),first.font_size),("Helvetica",18.));
        assert!((first.x-40.).abs()<0.01 && (first.y-(792.-700.-18.*0.8)).abs()<0.01 && first.width>100. && first.width<160.,"{first:?}");
        let picture=&runs[1];
        assert_eq!((picture.x,picture.y,picture.width,picture.height),(40.,142.,100.,50.));
    }

    #[test]
    fn edited_text_keeps_its_font_extracts_and_leaves_the_original_and_shared_stream_alone() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); text_edit_fixture(&input, false);
        let original=fs::read(&input).unwrap();
        let edits=serde_json::json!([{ "pageIndex": 1, "run": 0, "original": "Original content", "replacement": "Revised (content) é" }]);
        let request=edit_request(&input, edits);
        let preview=compose_with(&input,&request.scene,&request.sections).unwrap();
        let preview_id=preview.get_pages().values().copied().nth(1).unwrap();
        assert_eq!(text_edit::page_text(&preview,preview_id,[1.,0.,0.,1.,0.,0.]).unwrap().runs()[0].text,"Revised (content) é");

        let outcome=export(&request,input.clone());
        assert!(outcome.error().is_none(),"{:?}",outcome.error());
        assert_eq!(outcome.output_paths(),[temp.0.join("source-edited.pdf")]);
        assert_eq!(fs::read(&input).unwrap(),original);
        let output=&outcome.output_paths()[0];
        let edited=page_runs(output,1);
        assert_eq!((edited[0].text.as_str(),edited[0].font_name.as_str(),edited[0].font_size,edited[0].reason),("Revised (content) é","Helvetica",18.,None));
        assert_eq!(page_runs(output,0)[0].text,"Original content");
        let doc=load(output).unwrap();
        let id=doc.get_pages().values().copied().nth(1).unwrap();
        let content=String::from_utf8_lossy(&doc.get_page_content(id)).to_string();
        assert!(content.contains("/OriginalFont 18 Tf") && content.contains("1 0 0 rg"),"{content}");
        if text_extractor().is_some() {
            let runs=extract_text_runs(output,2,612.,792.,Instant::now() + super::super::PDF_TEXT_TIMEOUT).unwrap();
            let text=runs.iter().map(|run| run.text.as_str()).collect::<Vec<_>>().join(" ");
            assert!(text.contains("Revised (content) é") && !text.contains("Original content"),"{text}");
        }
    }

    #[test]
    fn text_edits_that_cannot_be_kept_faithful_are_refused_without_output() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); text_edit_fixture(&input, false);
        let embedded=temp.0.join("embedded.pdf"); text_edit_fixture(&embedded, true);
        let cases=[
            (&input, serde_json::json!([]), "No text was changed"),
            (&input, serde_json::json!([{ "pageIndex": 1, "run": 1, "original": "", "replacement": "x" }]), "This text is part of a picture"),
            (&input, serde_json::json!([{ "pageIndex": 1, "run": 2, "original": "", "replacement": "x" }]), "This text uses a font with no character map"),
            (&input, serde_json::json!([{ "pageIndex": 1, "run": 3, "original": "Tagged words", "replacement": "Other words" }]), "This text is linked to the PDF's accessibility tags"),
            (&input, serde_json::json!([{ "pageIndex": 1, "run": 0, "original": "Stale", "replacement": "x" }]), "The text you edited on page 2 no longer matches"),
            (&input, serde_json::json!([{ "pageIndex": 1, "run": 0, "original": "Original content", "replacement": "Arrow →" }]), "“→” isn't in the character set"),
            (&input, serde_json::json!([{ "pageIndex": 1, "run": 0, "original": "Original content", "replacement": "Two\nlines" }]), "Line breaks"),
            (&input, serde_json::json!([{ "pageIndex": 1, "run": 0, "original": "Original content", "replacement": "Original content" }]), "A text edit on page 2 doesn't change"),
            (&input, serde_json::json!([{ "pageIndex": 9, "run": 0, "original": "Original content", "replacement": "x" }]), "A text edit points to a page"),
            (&embedded, serde_json::json!([{ "pageIndex": 0, "run": 0, "original": "Original content", "replacement": "Original contents!" }]), "This text's font only includes the characters already used on page 1, so “!”"),
        ];
        for (path,edits,message) in cases {
            let original=fs::read(path).unwrap();
            let outcome=export(&edit_request(path, edits),path.clone());
            let error=outcome.error().expect("edit should be refused");
            assert!(error.kind==crate::kit::contracts::ErrorKind::InvalidInput && error.message.starts_with(message),"{message}: {error:?}");
            assert!(outcome.output_paths().is_empty());
            assert_eq!(fs::read_dir(&temp.0).unwrap().count(),2);
            assert_eq!(fs::read(path).unwrap(),original);
        }
        let outcome=export(&edit_request(&embedded, serde_json::json!([{ "pageIndex": 0, "run": 0, "original": "Original content", "replacement": "content origin" }])),embedded.clone());
        assert!(outcome.error().is_none(),"{:?}",outcome.error());
        assert_eq!(page_runs(&outcome.output_paths()[0],0)[0].text,"content origin");
    }

    #[test]
    fn password_protected_pdf_is_refused_with_a_file_security_pointer_and_no_output() {
        use crate::kit::contracts::ErrorKind;
        let temp=TempDir::new().unwrap(); let input=temp.0.join("locked.pdf"); fixture(&input);
        crate::kit::pdf::tests::encrypt_pdf(&input);
        let original=fs::read(&input).unwrap();

        let refusal=inspect(&input).err().expect("a locked PDF must not inspect");
        assert_eq!((refusal.kind,refusal.message.as_str(),refusal.pointer),(ErrorKind::InvalidInput,LOCKED_PDF_MESSAGE,Some(OutcomePointer::Tool { tool_id: "lock-unlock".into() })));
        let scene=PdfScene { pages:vec![ScenePage {source_index:Some(0),width:300.,height:200.,rotation:0,crop:None,source_rotation:None,source_box:None,objects:vec![object(Kind::Shape)]}] };
        let outcome=export(&ExportRequest {paths:vec![input.clone()],scene,output_location:OutputLocation::AlongsideInput, output_path: None, sections: Default::default()},input.clone());
        let error=outcome.error().expect("a locked PDF must fail");
        assert_eq!(error.kind,ErrorKind::InvalidInput);
        assert_eq!(error.message,LOCKED_PDF_MESSAGE);
        assert_eq!(error.pointer,Some(OutcomePointer::Tool { tool_id: "lock-unlock".into() }));
        assert!(outcome.output_paths().is_empty());
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(),1, "nothing may be written next to the locked original");
        assert_eq!(fs::read(&input).unwrap(),original);
    }

    fn identity_scene() -> PdfScene {
        PdfScene { pages: vec![
            ScenePage { source_index: Some(0), width: 300., height: 200., rotation: 0, crop: None, source_rotation: Some(90), source_box: Some(Rect { x: 20., y: 30., width: 200., height: 300. }), objects: vec![] },
            ScenePage { source_index: Some(1), width: 612., height: 792., rotation: 0, crop: None, source_rotation: Some(0), source_box: Some(Rect { x: 0., y: 0., width: 612., height: 792. }), objects: vec![] },
        ] }
    }

    fn export_request(input: &Path, scene: PdfScene, output_location: OutputLocation, output_path: Option<PathBuf>) -> ExportRequest {
        ExportRequest { paths: vec![input.to_path_buf()], scene, output_location, output_path, sections: Default::default() }
    }

    fn file_name(path: &Path) -> String { path.file_name().unwrap().to_string_lossy().into_owned() }

    #[test]
    fn export_suffix_names_the_single_contributing_stage_or_the_combined_edit() {
        assert_eq!(export_suffix(&[]), None);
        assert_eq!(export_suffix(&[Stage::Overlays]), Some("-edited"));
        assert_eq!(export_suffix(&[Stage::EditText]), Some("-edited"));
        assert_eq!(export_suffix(&[Stage::FillForms]), Some("-filled"));
        assert_eq!(export_suffix(&[Stage::Flatten]), Some("-flattened"));
        assert_eq!(export_suffix(&[Stage::Redact]), Some("-redacted"));
        assert_eq!(export_suffix(&[Stage::FillForms, Stage::Redact]), Some(COMBINED_SUFFIX));
        assert_eq!(export_suffix(&[Stage::FillForms, Stage::Overlays, Stage::Flatten]), Some("-edited"));
    }

    #[test]
    fn repeated_exports_take_the_suffix_then_number_without_overwriting_and_stay_byte_identical() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); fixture(&input);
        let original=fs::read(&input).unwrap();
        let request=export_request(&input, scene(), OutputLocation::AlongsideInput, None);
        let outcomes=(0..3).map(|_| export(&request,input.clone())).collect::<Vec<_>>();
        for outcome in &outcomes { assert!(outcome.error().is_none(),"{:?}",outcome.error()); assert!(outcome.warnings.is_empty()); }
        let names=outcomes.iter().map(|outcome| file_name(&outcome.output_paths()[0])).collect::<Vec<_>>();
        assert_eq!(names,["source-edited.pdf","source-edited-1.pdf","source-edited-2.pdf"]);
        assert!(outcomes.iter().all(|outcome| outcome.output_paths()[0].parent()==Some(temp.0.as_path())));
        let bytes=outcomes.iter().map(|outcome| fs::read(&outcome.output_paths()[0]).unwrap()).collect::<Vec<_>>();
        assert_eq!(bytes[0],bytes[1]); assert_eq!(bytes[1],bytes[2]);
        assert_eq!(fs::read(&input).unwrap(),original);
    }

    #[test]
    fn export_honours_a_custom_output_folder() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); fixture(&input);
        let original=fs::read(&input).unwrap();
        let folder=temp.0.join("exports"); fs::create_dir(&folder).unwrap();
        let request=export_request(&input, scene(), OutputLocation::CustomFolder(folder.clone()), None);
        let first=export(&request,input.clone()); let second=export(&request,input.clone());
        assert_eq!(first.output_paths(),[folder.join("source-edited.pdf")]);
        assert_eq!(second.output_paths(),[folder.join("source-edited-1.pdf")]);
        assert!(!temp.0.join("source-edited.pdf").exists());
        assert_eq!(fs::read(&input).unwrap(),original);
    }

    #[test]
    fn export_as_writes_the_chosen_file_and_numbers_a_taken_name_with_the_shared_warning() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); fixture(&input);
        let original=fs::read(&input).unwrap();
        let chosen=temp.0.join("contract final.pdf");
        let fresh=export(&export_request(&input, scene(), OutputLocation::AlongsideInput, Some(chosen.clone())),input.clone());
        assert_eq!(fresh.output_paths(),[chosen.clone()]);
        assert!(fresh.warnings.is_empty());
        let existing=fs::read(&chosen).unwrap();
        let renamed=export(&export_request(&input, scene(), OutputLocation::AlongsideInput, Some(chosen.clone())),input.clone());
        assert_eq!(renamed.output_paths(),[temp.0.join("contract final-1.pdf")]);
        assert!(matches!(&renamed.warnings[..], [warning] if matches!(&warning.kind, crate::kit::contracts::WarningKind::RenamedToAvoidReplacing { chosen_name, final_name } if chosen_name=="contract final.pdf" && final_name=="contract final-1.pdf")), "{:?}", renamed.warnings);
        assert_eq!(fs::read(&chosen).unwrap(),existing);
        assert_eq!(fs::read(&input).unwrap(),original);
    }

    #[test]
    fn export_as_rejects_a_batch_before_output() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); fixture(&input);
        let mut request=export_request(&input, scene(), OutputLocation::AlongsideInput, Some(temp.0.join("chosen.pdf")));
        request.paths.push(input.clone());
        let outcome=export(&request,input.clone());
        assert!(outcome.error().is_some_and(|error| error.kind==crate::kit::contracts::ErrorKind::InvalidInput),"{outcome:?}");
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(),1);
    }

    #[test]
    fn a_request_without_an_effective_change_is_refused_as_invalid_input_with_no_output() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); fixture(&input);
        let original=fs::read(&input).unwrap();
        for output_path in [None, Some(temp.0.join("chosen.pdf"))] {
            let outcome=export(&export_request(&input, identity_scene(), OutputLocation::AlongsideInput, output_path),input.clone());
            let error=outcome.error().expect("no-change export must fail");
            assert_eq!(error.kind,crate::kit::contracts::ErrorKind::InvalidInput);
            assert_eq!(error.message,NOTHING_TO_EXPORT);
            assert!(outcome.output_paths().is_empty());
            assert_eq!(fs::read_dir(&temp.0).unwrap().count(),1);
        }
        assert_eq!(fs::read(&input).unwrap(),original);
    }
}
