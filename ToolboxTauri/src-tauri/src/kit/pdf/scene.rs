use std::{collections::{HashMap, HashSet}, fs::{self, OpenOptions}, io::Write, path::{Path, PathBuf}, process::Command, sync::atomic::{AtomicU64, Ordering}, time::{Instant}};
use base64::Engine;
use rustybuzz::ttf_parser;
use flate2::{write::ZlibEncoder, Compression};
use lopdf::{dictionary, Document, Object, ObjectId, Stream};
use quick_xml::{events::Event, escape::unescape, Reader as XmlReader, XmlVersion};
use serde::{Deserialize, Serialize};
use crate::kit::{common::{limits, watermark::{WatermarkOptions, WatermarkPlacement, WatermarkPosition}, JobOutcome, OutputLocation, OutputNaming}, contracts::{LimitKind, ToolError}};
use super::metadata::{PdfDocumentMetadata, PdfPageMetadata, PdfTextRun};
use super::{inherited, resolve, PdfMutationIntent};

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
    #[serde(default)] pub page_numbers: Option<PageNumberOptions>,
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all="kebab-case")]
pub enum PageNumberFormat { Number, Page, PageOfTotal, NumberOfTotal, Dashes }
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all="kebab-case")]
pub enum PageNumberPosition { TopLeft, TopCenter, TopRight, MiddleLeft, MiddleCenter, MiddleRight, BottomLeft, BottomCenter, BottomRight }
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all="camelCase")]
pub struct PageNumberOptions { pub format: PageNumberFormat, pub start: u32, pub position: PageNumberPosition, pub margin: f32 }
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all="lowercase")]
pub enum TextAlign { #[default] Left, Center, Right }
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all="camelCase")]
pub enum Kind { Text, Highlight, Shape, Signature, Watermark, PageNumbers }
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
pub struct PreviewRequest { pub path: PathBuf, pub scene: PdfScene, pub page_index: usize }
#[derive(Deserialize)]
#[serde(rename_all="camelCase")]
pub struct PreviewPagesRequest { pub path: PathBuf, pub scene: PdfScene, pub page_indices: Vec<usize> }
#[derive(Deserialize)]
#[serde(rename_all="camelCase")]
pub struct ExportRequest { pub paths: Vec<PathBuf>, pub scene: PdfScene, pub output_location: OutputLocation, #[serde(flatten)] pub sections: StageSections }
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all="camelCase")]
pub struct StageSections {
    pub form_values: Option<Vec<serde_json::Value>>,
    pub text_edits: Option<Vec<serde_json::Value>>,
    pub flatten: Option<bool>,
    pub redactions: Option<Vec<serde_json::Value>>,
}
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
fn load(path:&Path)->Result<Document,String> {
    let d=Document::load(path).map_err(err)?;
    if d.is_encrypted() { return Err("Unlock the PDF before editing".into()); } Ok(d)
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

fn unrenderable_message(family: &str, missing: &[char], target: &str) -> String {
    let named = missing.iter().take(10).map(|c| format!("“{c}” (U+{:04X})", *c as u32)).collect::<Vec<_>>().join(", ");
    let more = if missing.len() > 10 { format!(" and {} more", missing.len() - 10) } else { String::new() };
    let noun = if missing.len() == 1 { "this character" } else { "these characters" };
    format!("{family} can't show {noun}: {named}{more}. Remove {} to export {target}.", if missing.len() == 1 { "it" } else { "them" })
}

fn layout_text(o: &SceneObject) -> Result<TextLayout, ToolError> {
    layout_lines(text_font(o)?, &o.text, o.font_size, o.rect.width + 0.01)
}

fn layout_lines(font: TextFont, text: &str, font_size: f32, max_width: f32) -> Result<TextLayout, ToolError> {
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
    if !missing.is_empty() { return Err(ToolError::invalid_input(unrenderable_message(font.family, &missing, "this text box"))); }
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

const PAGE_NUMBER_SIZES: std::ops::RangeInclusive<f32> = 8.0..=72.0;
const PAGE_NUMBER_MAX_MARGIN: f32 = 144.;

impl PageNumberFormat {
    fn template(self) -> &'static str {
        match self { Self::Number => "{page}", Self::Page => "Page {page}", Self::PageOfTotal => "Page {page} of {pages}", Self::NumberOfTotal => "{page} / {pages}", Self::Dashes => "- {page} -" }
    }
}

/// Mirrors resolvePageTokens in the editor.
fn page_tokens(template: &str, page: u64, pages: u64) -> String {
    template.replace("{pages}", &pages.to_string()).replace("{page}", &page.to_string())
}

fn page_number_options(o: &SceneObject) -> Result<&PageNumberOptions, ToolError> {
    let options = o.page_numbers.as_ref().ok_or_else(|| ToolError::invalid_input("Choose a page-number format and position."))?;
    if !PAGE_NUMBER_SIZES.contains(&o.font_size) { return Err(ToolError::invalid_input("Page numbers must be between 8 and 72 pt.")); }
    if options.start == 0 { return Err(ToolError::invalid_input("Page numbers must start at 1 or higher.")); }
    if !options.margin.is_finite() || !(0. ..=PAGE_NUMBER_MAX_MARGIN).contains(&options.margin) {
        return Err(ToolError::invalid_input("The distance from the edge must be between 0 and 2 inches."));
    }
    Ok(options)
}

/// Places a page number of the given size inside the visible area, `margin` points from the chosen edges.
fn page_number_rect(area: &Rect, options: &PageNumberOptions, width: f32, height: f32) -> Rect {
    use PageNumberPosition::*;
    let (fx, fy) = match options.position {
        TopLeft => (0., 0.), TopCenter => (0.5, 0.), TopRight => (1., 0.),
        MiddleLeft => (0., 0.5), MiddleCenter => (0.5, 0.5), MiddleRight => (1., 0.5),
        BottomLeft => (0., 1.), BottomCenter => (0.5, 1.), BottomRight => (1., 1.),
    };
    let m = options.margin;
    Rect { x: area.x + m + (area.width - 2. * m - width) * fx, y: area.y + m + (area.height - 2. * m - height) * fy, width, height }
}

/// A text layout per object; page numbers have one per page they are drawn on.
type LayoutKey = (usize, usize, Option<usize>);
struct TextPlan { placed: Vec<Vec<(usize, usize)>>, layouts: HashMap<LayoutKey, TextLayout>, signatures: HashMap<(usize, usize), SignatureImage> }
impl TextPlan {
    fn layout(&self, (origin, index): (usize, usize), page: usize) -> Option<&TextLayout> {
        self.layouts.get(&(origin, index, Some(page))).or_else(|| self.layouts.get(&(origin, index, None)))
    }
}

fn text_plan(scene: &PdfScene) -> Result<TextPlan, ToolError> {
    let mut placed = vec![Vec::new(); scene.pages.len()];
    let mut layouts = HashMap::new();
    let mut signatures = HashMap::new();
    let page_count = scene.pages.len();
    for (origin, page) in scene.pages.iter().enumerate() {
        for (index, o) in page.objects.iter().enumerate() {
            let targets = match (&o.kind, o.page_range.as_deref()) {
                (Kind::Text | Kind::Signature | Kind::Watermark | Kind::PageNumbers, Some(range)) => page_range_pages(range, page_count)?,
                (_, Some(_)) => return Err(ToolError::invalid_input("Only text boxes, signatures, watermarks and page numbers can be shown on several pages")),
                _ => vec![origin],
            };
            for &target in &targets { placed[target].push((origin, index)); }
            match o.kind {
                Kind::Text => { layouts.insert((origin, index, None), layout_text(o)?); }
                Kind::Signature => { signatures.insert((origin, index), signature(o)?); }
                Kind::Watermark => {
                    let options = watermark_options(o)?;
                    if let Some(path) = &options.logo_path { check_watermark_image(path)?; }
                    // Laid out at 1 pt without wrapping; each page scales it to its own size.
                    if let Some(text) = options.text() { layouts.insert((origin, index, None), layout_lines(text_font(o)?, text, 1., f32::INFINITY)?); }
                }
                Kind::PageNumbers => {
                    let options = page_number_options(o)?;
                    let font = text_font(o)?;
                    // Numbers follow the final page order: the page at `target` shows start + target.
                    let start = u64::from(options.start);
                    for &target in &targets {
                        let text = page_tokens(options.format.template(), start + target as u64, start + page_count as u64 - 1);
                        layouts.insert((origin, index, Some(target)), layout_lines(font, &text, o.font_size, f32::INFINITY)?);
                    }
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
fn embed_text_fonts(doc: &mut Document, layouts: &HashMap<LayoutKey, TextLayout>) -> Result<HashMap<&'static str, (ObjectId, subsetter::GlyphRemapper)>, String> {
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

fn text_content(r: &Rect, align: TextAlign, font_size: f32, layout: &TextLayout, resource: &str, remapper: &subsetter::GlyphRemapper) -> String {
    let slant = if layout.font.oblique { SYNTHETIC_OBLIQUE } else { 0. };
    let mut content = format!("{} {} {} {} re W n\nBT /{resource} {} Tf\n", r.x, r.y, r.width, r.height, font_size);
    for (index, line) in layout.lines.iter().enumerate() {
        if line.glyphs.is_empty() { continue; }
        let x = r.x + match align { TextAlign::Left => 0., TextAlign::Center => (r.width - line.width) / 2., TextAlign::Right => r.width - line.width };
        let y = r.y + layout.first_baseline + index as f32 * font_size * TEXT_LINE_HEIGHT;
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

fn add_image_xobject(document: &mut Document, image: &SignatureImage) -> ObjectId {
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
    let common = super::mutation_preflight(document, PdfMutationIntent::ValidateOnly)?;
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

struct OverlayPlan { preflight: ScenePreflight, identity: bool, pure_reorder: bool }
enum StagePlan { Skip, Overlays(OverlayPlan) }

fn unsupported_section(present: bool, name: &str) -> Result<StagePlan, String> {
    if present { return Err(format!("{name} is not supported yet; scene export was rejected before output")); }
    Ok(StagePlan::Skip)
}

fn overlay_preflight(doc: &Document, scene: &PdfScene) -> Result<StagePlan, String> {
    if scene.pages.is_empty() || scene.pages.len()>2000 { return Err("Scene must contain 1–2000 pages".into()); }
    let sources:Vec<_>=doc.get_pages().values().copied().collect();
    let identity = is_identity_scene(doc, scene, &sources)?;
    let pure_reorder = is_pure_page_reorder(doc, scene, &sources)?;
    let preflight = scene_preflight(doc, scene, &sources, identity, pure_reorder)?;
    Ok(StagePlan::Overlays(OverlayPlan { preflight, identity, pure_reorder }))
}

fn stage_preflight(stage: Stage, doc: &Document, scene: &PdfScene, sections: &StageSections) -> Result<StagePlan, String> {
    match stage {
        Stage::FillForms => unsupported_section(sections.form_values.is_some(), "Filling form fields"),
        Stage::EditText => unsupported_section(sections.text_edits.is_some(), "Editing existing text"),
        Stage::Overlays => overlay_preflight(doc, scene),
        Stage::Flatten => unsupported_section(sections.flatten.is_some(), "Flattening"),
        Stage::Redact => unsupported_section(sections.redactions.is_some(), "Redaction"),
    }
}

fn stage_apply(plan: StagePlan, doc: &mut Document, scene: &PdfScene) -> Result<(), String> {
    match plan {
        StagePlan::Skip => Ok(()),
        StagePlan::Overlays(plan) => apply_overlays(doc, scene, plan),
    }
}

fn run_stages(path:&Path, scene:&PdfScene, sections:&StageSections)->Result<Document,String> {
    let mut doc=load(path)?;
    let plans=STAGES.iter().map(|&stage| stage_preflight(stage, &doc, scene, sections)).collect::<Result<Vec<_>,String>>()?;
    for plan in plans { stage_apply(plan, &mut doc, scene)?; }
    Ok(doc)
}

pub fn compose(path:&Path, scene:&PdfScene)->Result<Document,String> { run_stages(path, scene, &StageSections::default()) }

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
                pictures.insert((origin, index), (add_image_xobject(doc, &image), size));
            }
        }
    }
    let mut signature_ids = HashMap::new();
    let mut kids=Vec::new();
    for (page_index, page) in scene.pages.iter().enumerate() {
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
        for key in placed { if let Some(layout) = text.layout(*key, page_index) {
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
                    let id = *signature_ids.entry(*key).or_insert_with(|| add_image_xobject(doc, image));
                    let name = next_resource_name(&effective_xobjects, "XObject", &format!("Sig{i}"))?;
                    effective_xobjects.set(name.as_str(), Object::Null);
                    xobjects.set(name.as_str(), id);
                    let scale = (r.width / image.width as f32).min(r.height / image.height as f32);
                    let (w, h) = (image.width as f32 * scale, image.height as f32 * scale);
                    let (x, y) = (r.x + (r.width - w) / 2., r.y + (r.height - h) / 2.);
                    content.push_str(&format!("q {w} 0 0 {} {x} {} cm /{name} Do Q\n", -h, y + h));
                },
                Kind::Text=>{
                    let layout = text.layout(*key, page_index).ok_or("Text layout disappeared")?;
                    content.push_str(&text_content(&o.rect, o.align, o.font_size, layout, &text_resources[layout.font.name], &text_fonts[layout.font.name].1));
                },
                Kind::PageNumbers=>{
                    let options = o.page_numbers.as_ref().ok_or("Page-number options disappeared")?;
                    let layout = text.layout(*key, page_index).ok_or("Page-number layout disappeared")?;
                    let width = layout.lines.iter().map(|line| line.width).fold(0., f32::max);
                    let rect = page_number_rect(&area, options, width, o.font_size * TEXT_LINE_HEIGHT);
                    content.push_str(&text_content(&rect, TextAlign::Left, o.font_size, layout, &text_resources[layout.font.name], &text_fonts[layout.font.name].1));
                },
                Kind::Watermark=>{
                    let options = watermark.ok_or("Watermark options disappeared")?;
                    let layout = text.layout(*key, page_index);
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
    std::env::var_os("TOOLBOX_PDFTOTEXT_PATH").map(PathBuf::from).filter(|path|path.is_file())
        .or_else(|| crate::kit::resources::application_resource_root().and_then(|root| [root.join("pdf-bin").join("pdftotext"), root.join("resources").join("pdftotext"), root.join("pdftotext")].into_iter().find(|path|path.is_file())))
        .or_else(|| {
            let mut command = Command::new("pdftotext");
            command.arg("-h");
            super::helper_available(command).then(|| PathBuf::from("pdftotext"))
        })
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
    compose(&request.path,&request.scene)?.save(&path).map_err(err)?;
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
    compose(&request.path,&request.scene)?.save(&path).map_err(err)?;
    request.page_indices.iter().copied().map(|page_index| {
        render(&path,page_index,&renderer,&temp).map(|preview| IndexedScenePreview{page_index,preview})
    }).collect()
}
pub fn inspect(path:&Path)->Result<PdfDocumentMetadata,String> {
    let doc=load(path)?;
    let mut pages=Vec::new();
    for (index,id) in doc.get_pages().values().enumerate() {
        let g=geometry(&doc,*id)?;
        let source_rotation = inherited(&doc, *id, b"Rotate")?.map(|value| value.as_i64().map_err(err)).transpose()?.unwrap_or(0).rem_euclid(360) as i32;
        pages.push(PdfPageMetadata{index,x:g.bbox[0],y:g.bbox[1],width:g.width,height:g.height,rotation:source_rotation,page_box:g.bbox,preview:None,text_runs:None});
    }
    if pages.is_empty() { return Err("PDF contains no pages".into()); }
    Ok(PdfDocumentMetadata{path:path.to_path_buf(),pages})
}
pub fn inspect_page(path:&Path,page_index:usize)->Result<PdfPageMetadata,String> {
    let doc=load(path)?;
    let pages:Vec<_>=doc.get_pages().values().copied().collect();
    let id=*pages.get(page_index).ok_or("PDF page outside document")?;
    let g=geometry(&doc,id)?;
    let source_rotation = inherited(&doc, id, b"Rotate")?.map(|value| value.as_i64().map_err(err)).transpose()?.unwrap_or(0).rem_euclid(360) as i32;
    let text_runs=extract_text_runs(path,page_index + 1,g.width,g.height,Instant::now() + super::PDF_TEXT_TIMEOUT).unwrap_or_default();
    let preview = super::metadata::render_preview(path, page_index + 1, g.width, g.height);
    Ok(PdfPageMetadata{index:page_index,x:g.bbox[0],y:g.bbox[1],width:g.width,height:g.height,rotation:source_rotation,page_box:g.bbox,preview,text_runs:Some(text_runs)})
}
pub fn export(request:&ExportRequest,input:PathBuf)->JobOutcome {
    let result=(|| {
        let processing=|error:String| ToolError::processing(error);
        if !matches!(request.output_location,OutputLocation::AlongsideInput) { return Err(processing("Scene exports must be alongside the input".into())); }
        text_plan(&request.scene)?;
        let mut bytes=Vec::new();
        run_stages(&input,&request.scene,&request.sections).map_err(processing)?.save_to(&mut bytes).map_err(|error| processing(err(error)))?;
        let parent=input.parent().ok_or_else(|| processing("Input has no parent directory".into()))?;
        let stem=input.file_stem().ok_or_else(|| processing("Input has no filename".into()))?.to_string_lossy();
        for n in 1..=10000 {
            let candidate=parent.join(format!("{stem}-edited-{n}.pdf"));
            let reservation=match OutputNaming::reserve_named_candidate(&candidate).map_err(|error| processing(err(error)))? {
                Some(reservation)=>reservation,
                None=>continue,
            };
            let mut file=OpenOptions::new().write(true).open(reservation.path()).map_err(|error| processing(err(error)))?;
            file.write_all(&bytes).and_then(|_|file.sync_all()).map_err(|error| processing(err(error)))?;
            return reservation.publish().map_err(|error| processing(err(error)));
        } Err(processing("Could not reserve a unique export filename".into()))
    })();
    match result { Ok(path)=>JobOutcome::succeeded(input, vec![path], "PDF scene exported"),Err(e)=>JobOutcome::failure(input,e) }
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
        let fonts_id = d.add_object(dictionary! { "SceneFont" => existing_font, "SceneFont1" => existing_font });
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
        SceneObject { signature_mode, rect: Rect{x:30.,y:40.,width:120.,height:40.}, text:"LOCAL".into(),font_size:18.,color:"#0066CC".into(),opacity:0.5,
            shape:None, highlight_mode:None,
            signature_path:None, font_family:None,
            watermark: matches!(kind, Kind::Watermark).then(|| text_watermark("LOCAL")), layer: Layer::Front,
            bold:false, italic:false, align:TextAlign::Left, page_range:None, page_numbers:None, kind }
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
        let outcome = export(&ExportRequest { paths: vec![input.clone()], scene, output_location: OutputLocation::AlongsideInput, sections: Default::default() }, input.clone());
        assert!(outcome.error().is_none(), "no-op export failed: {:?}", outcome.error());
        let output = outcome.output_paths().first().unwrap();
        let output_document = Document::load(output).unwrap();
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
        let outcome = export(&ExportRequest { paths: vec![input.clone()], scene, output_location: OutputLocation::AlongsideInput, sections: Default::default() }, input.clone());
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
        let occupied=temp.0.join("source-edited-1.pdf"); fs::write(&occupied,b"existing output").unwrap();
        let request=ExportRequest {paths:vec![input.clone()],scene:scene(),output_location:OutputLocation::AlongsideInput, sections: Default::default()};
        let a=export(&request,input.clone()); let b=export(&request,input.clone());
        assert!(a.error().is_none(),"{:?}",a.error()); assert!(b.error().is_none());
        assert_ne!(a.output_paths(),b.output_paths()); assert_ne!(a.output_paths()[0],input);
        assert_eq!(fs::read(&input).unwrap(),original); assert_eq!(fs::read(occupied).unwrap(),b"existing output");
        assert_eq!(Document::load(&a.output_paths()[0]).unwrap().get_pages().len(),3);
    }
    #[test]
    fn scene_reservation_does_not_replace_a_competing_destination() {
        let temp=TempDir::new().unwrap();
        let candidate=temp.0.join("source-edited-1.pdf");
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
        let result=export(&ExportRequest {paths:vec![],scene:s,output_location:OutputLocation::AlongsideInput, sections: Default::default()},input.clone());
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
            let result=preview_pages(&PreviewPagesRequest { path: PathBuf::from("missing.pdf"), scene: scene(), page_indices });
            assert_eq!(result.err().as_deref(),Some(message));
        }
    }

    #[test]
    fn preview_pages_returns_ordered_indexed_previews_from_one_normalized_scene() {
        let _guard = crate::kit::lock_process_env();
        let renderer=match renderer() { Ok(r)=>r,Err(e)=>{if std::env::var_os("TOOLBOX_REQUIRE_PDF_RENDERER").is_some(){panic!("{e}")}; eprintln!("Renderer unavailable: {e}");return;} };
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); fixture(&input);
        let before=fs::read(&input).unwrap();
        let request=PreviewPagesRequest { path:input.clone(), scene:scene(), page_indices:vec![2,0,1] };
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
        let request=ExportRequest{paths:vec![input.clone()],scene:scene(),output_location:OutputLocation::AlongsideInput, sections: Default::default()};
        let exported=export(&request,input.clone()); assert!(exported.error().is_none());
        for index in 0..3 {
            let preview=preview(&PreviewRequest{path:input.clone(),scene:scene(),page_index:index}).unwrap();
            let actual=render(&exported.output_paths()[0],index,&renderer,&temp).unwrap();
            assert_eq!(preview.data_url,actual.data_url,"page {index} differs between preview and export");
        }
        assert_eq!(fs::read(&input).unwrap(),before);
        let directory={let guard=TempDir::new().unwrap();let p=guard.0.clone();fs::write(p.join("private.pdf"),b"temporary").unwrap();p};
        assert!(!directory.exists());
        // Removing all source marks must visibly differ: negative control for the pixel oracle.
        let mut bare=scene();bare.pages[0].objects.clear();
        assert_ne!(preview(&PreviewRequest{path:input.clone(),scene:bare,page_index:0}).unwrap().data_url,
            preview(&PreviewRequest{path:input,scene:scene(),page_index:0}).unwrap().data_url);
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
        assert!(content.contains("/S02 gs") && content.contains("/S12 gs"), "state resource collisions: {content}");
        assert!(content.contains("/Original2 Do") && content.contains("/Sig12 Do"), "xobject resource collisions: {content}");

        let resources = composed.get_dictionary(page).unwrap().get(b"Resources").unwrap().as_dict().unwrap();
        let fonts = resources.get(b"Font").unwrap().as_dict().unwrap();
        let xobjects = resources.get(b"XObject").unwrap().as_dict().unwrap();
        let states = resources.get(b"ExtGState").unwrap().as_dict().unwrap();
        for name in ["SceneFont", "SceneFont1", "SceneFont2"] { assert!(fonts.has(name.as_bytes())); }
        for name in ["Original", "Original1", "Original2", "Sig1", "Sig11", "Sig12"] { assert!(xobjects.has(name.as_bytes())); }
        for name in ["S0", "S01", "S02", "S1", "S11", "S12"] { assert!(states.has(name.as_bytes())); }
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
        let result=export(&ExportRequest {paths:vec![input.clone()],scene,output_location:OutputLocation::AlongsideInput, sections: Default::default()},input.clone());
        assert!(result.error().is_some_and(|error| error.message.contains("nested")));
        assert!(result.output_paths().is_empty());
        assert!(!input.with_file_name("nested-edited-1.pdf").exists());
    }
    #[test]
    fn scene_rejects_annotation_crop_before_output() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("annotated.pdf"); fixture(&input);
        let mut source=Document::load(&input).unwrap(); let page=source.get_pages().values().next().copied().unwrap();
        let annotation=source.add_object(dictionary! {"Type"=>"Annot", "Subtype"=>"Text", "Rect"=>array(&[20.,20.,80.,50.])});
        source.get_dictionary_mut(page).unwrap().set("Annots",vec![Object::Reference(annotation)]); source.save(&input).unwrap();
        let scene=PdfScene { pages:vec![ScenePage { source_index:Some(0), width:300., height:200., rotation:0, crop:Some(Rect{x:0.,y:0.,width:100.,height:100.}), source_rotation:None, source_box:None, objects:vec![] }] };
        let result=export(&ExportRequest {paths:vec![input.clone()],scene,output_location:OutputLocation::AlongsideInput, sections: Default::default()},input.clone());
        assert!(result.error().is_some_and(|error| error.message.contains("annotation")));
        assert!(result.output_paths().is_empty());
        assert!(!input.with_file_name("annotated-edited-1.pdf").exists());
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
        let result=export(&ExportRequest {paths:vec![input.clone()],scene,output_location:OutputLocation::AlongsideInput, sections: Default::default()},input.clone());
        assert!(result.error().is_some_and(|error| error.message.contains("Digitally signed")));
        assert!(result.output_paths().is_empty());
        assert!(!input.with_file_name("signed-edited-1.pdf").exists());
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
            let result=export(&ExportRequest {paths:vec![input.clone()],scene,output_location:OutputLocation::AlongsideInput, sections: Default::default()},input.clone());
            assert!(result.error().is_some_and(|error| error.message.contains("Digitally signed")), "{name}: {:?}", result.error());
            assert!(result.output_paths().is_empty());
            assert!(!input.with_file_name(format!("{name}-edited-1.pdf")).exists());
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
        let result=export(&ExportRequest {paths:vec![input.clone()],scene,output_location:OutputLocation::AlongsideInput, sections: Default::default()},input.clone());
        assert!(result.error().is_some_and(|error| error.message.contains("destinations")), "{:?}", result.error());
        assert!(result.output_paths().is_empty());
        assert!(!input.with_file_name("navigation-edited-1.pdf").exists());
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
        let result=export(&ExportRequest {paths:vec![input.clone()],scene,output_location:OutputLocation::AlongsideInput, sections: Default::default()},input.clone());
        assert!(result.error().is_none(), "{result:?}");
        assert_eq!(result.output_paths().len(), 1);
        assert!(input.with_file_name("box-origin-navigation-edited-1.pdf").exists());
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
        let result=export(&ExportRequest {paths:vec![input.clone()],scene,output_location:OutputLocation::AlongsideInput, sections: Default::default()},input.clone());
        assert!(result.error().is_some_and(|error| error.message.contains("tagged PDF")), "{:?}", result.error());
        assert!(result.output_paths().is_empty());
        assert!(!input.with_file_name("tagged-edited-1.pdf").exists());
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
        let result = export(&ExportRequest { paths: vec![input.clone()], scene: overlay, output_location: OutputLocation::AlongsideInput, sections: Default::default() }, input.clone());
        assert!(result.error().is_some_and(|error| error.message.contains("tagged PDF")), "{result:?}");
        assert!(result.output_paths().is_empty());
        assert!(!input.with_file_name("tagged-overlay-edited-1.pdf").exists());
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
            (serde_json::json!({ "formValues": [] }), "Filling form fields is not supported yet"),
            (serde_json::json!({ "textEdits": [{ "run": 0 }] }), "Editing existing text is not supported yet"),
            (serde_json::json!({ "flatten": false }), "Flattening is not supported yet"),
            (serde_json::json!({ "redactions": [] }), "Redaction is not supported yet"),
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
            run_stages(&input,&request.scene,&request.sections).err().unwrap()
        };
        let all=serde_json::json!({ "formValues": [], "textEdits": [], "flatten": true, "redactions": [] });
        assert!(first_failure(all, None).starts_with("Filling form fields"));
        assert!(first_failure(serde_json::json!({ "textEdits": [], "flatten": true, "redactions": [] }), None).starts_with("Editing existing text"));
        assert!(first_failure(serde_json::json!({ "flatten": true, "redactions": [] }), Some(PdfScene{pages:vec![]})).starts_with("Scene must contain"));
        assert!(first_failure(serde_json::json!({ "flatten": true, "redactions": [] }), None).starts_with("Flattening"));
        assert!(first_failure(serde_json::json!({ "redactions": [] }), None).starts_with("Redaction"));
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
        export(&ExportRequest { paths: vec![input.to_path_buf()], scene, output_location: OutputLocation::AlongsideInput, sections: Default::default() }, input.to_path_buf())
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
            let preview = preview(&PreviewRequest { path: input.clone(), scene: text_scene(objects.clone()), page_index: index }).unwrap();
            let actual = render(&outcome.output_paths()[0], index, &renderer, &temp).unwrap();
            assert_eq!(preview.data_url, actual.data_url, "page {index} differs between preview and export");
        }
        let blank = preview(&PreviewRequest { path: input.clone(), scene: text_scene(vec![]), page_index: 2 }).unwrap();
        let ranged = preview(&PreviewRequest { path: input.clone(), scene: text_scene(objects), page_index: 2 }).unwrap();
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
        assert_eq!(compose(&input, &text_scene(vec![highlight])).unwrap_err(), "Only text boxes, signatures, watermarks and page numbers can be shown on several pages");
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
            let content = text_content(&o.rect, o.align, o.font_size, &layout, "F", &remapper);
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

    fn page_number_object(format: PageNumberFormat, position: PageNumberPosition, start: u32, range: &str) -> SceneObject {
        SceneObject { font_size: 12., color: "#202020".into(), opacity: 1., page_range: Some(range.into()),
            page_numbers: Some(PageNumberOptions { format, start, position, margin: 36. }), ..object(Kind::PageNumbers) }
    }
    fn page_words(extractor: &Path, path: &Path, page: usize) -> String {
        let output = Command::new(extractor).args(["-enc", "UTF-8", "-f"]).arg(page.to_string()).arg("-l").arg(page.to_string()).arg(path).arg("-").output().unwrap();
        String::from_utf8(output.stdout).unwrap().split_whitespace().collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn page_numbers_follow_the_final_page_order_and_clamp_ranges() {
        let Some(extractor) = text_extractor() else { return };
        let temp = TempDir::new().unwrap(); let input = temp.0.join("source.pdf"); page_text_fixture(&input);
        // Organize moved the first source page to the end; the number object stays on its origin page.
        let mut scene = text_scene(vec![page_number_object(PageNumberFormat::PageOfTotal, PageNumberPosition::BottomCenter, 1, "2-99")]);
        scene.pages.rotate_left(1);
        let outcome = export_text(&input, scene);
        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        let output = &outcome.output_paths()[0];
        let pages: Vec<_> = (1..=3).map(|page| page_words(&extractor, output, page)).collect();
        assert_eq!(pages[0], "Second page text", "page 1 is outside 2-99");
        assert_eq!(pages[1], "Page 2 of 3");
        assert_eq!(pages[2], "First page text Page 3 of 3");
    }

    #[test]
    fn page_number_formats_positions_and_start_numbers_render_as_configured() {
        if text_extractor().is_none() { return; }
        use PageNumberFormat::*; use PageNumberPosition::*;
        let temp = TempDir::new().unwrap(); let input = temp.0.join("source.pdf"); page_text_fixture(&input);
        let cases = [
            (Number, TopLeft, 1, "3"), (Page, TopCenter, 5, "Page 7"), (PageOfTotal, TopRight, 1, "Page 3 of 3"),
            (NumberOfTotal, MiddleLeft, 10, "12 / 12"), (Dashes, MiddleCenter, 1, "- 3 -"), (Number, MiddleRight, 1, "3"),
            (Page, BottomLeft, 1, "Page 3"), (PageOfTotal, BottomCenter, 3, "Page 5 of 5"), (Number, BottomRight, 99, "101"),
        ];
        for (format, position, start, expected) in cases {
            let outcome = export_text(&input, text_scene(vec![page_number_object(format, position, start, "3")]));
            assert!(outcome.error().is_none(), "{:?}", outcome.error());
            // The blank third page carries only the number.
            let runs = extract_text_runs(&outcome.output_paths()[0], 3, 612., 792., Instant::now() + super::super::PDF_TEXT_TIMEOUT).unwrap();
            assert_eq!(runs.iter().map(|run| run.text.as_str()).collect::<Vec<_>>().join(" "), expected, "{position:?}");
            let left = runs.iter().map(|run| run.x).fold(f32::MAX, f32::min);
            let right = runs.iter().map(|run| run.x + run.width).fold(0., f32::max);
            let top = runs.iter().map(|run| run.y).fold(f32::MAX, f32::min);
            let bottom = runs.iter().map(|run| run.y + run.height).fold(0., f32::max);
            let (fx, fy) = match position { TopLeft => (0, 0), TopCenter => (1, 0), TopRight => (2, 0), MiddleLeft => (0, 1), MiddleCenter => (1, 1), MiddleRight => (2, 1), BottomLeft => (0, 2), BottomCenter => (1, 2), BottomRight => (2, 2) };
            let near = |a: f32, b: f32| (a - b).abs() < 4.;
            match fx { 0 => assert!(near(left, 36.), "{position:?} left {left}"), 1 => assert!(near((left + right) / 2., 306.), "{position:?} centre"), _ => assert!(near(right, 576.), "{position:?} right {right}") }
            match fy { 0 => assert!(near(top, 36.), "{position:?} top {top}"), 1 => assert!(near((top + bottom) / 2., 396.), "{position:?} middle"), _ => assert!(near(bottom, 756.), "{position:?} bottom {bottom}") }
        }
    }

    #[test]
    fn page_numbers_refuse_out_of_range_settings_in_plain_language() {
        let temp = TempDir::new().unwrap(); let input = temp.0.join("source.pdf"); page_text_fixture(&input);
        let base = || page_number_object(PageNumberFormat::Number, PageNumberPosition::BottomRight, 1, "all");
        let refusal = |o: SceneObject| export_text(&input, text_scene(vec![o])).error().map(|e| e.to_string()).unwrap_or_default();
        assert!(refusal(SceneObject { font_size: 7., ..base() }).contains("between 8 and 72 pt"));
        assert!(refusal(SceneObject { font_size: 73., ..base() }).contains("between 8 and 72 pt"));
        let mut zero = base(); zero.page_numbers.as_mut().unwrap().start = 0;
        assert!(refusal(zero).contains("start at 1 or higher"));
        let mut far = base(); far.page_numbers.as_mut().unwrap().margin = 145.;
        assert!(refusal(far).contains("between 0 and 2 inches"));
        assert!(refusal(SceneObject { page_numbers: None, ..base() }).contains("Choose a page-number format"));
        assert!(refusal(SceneObject { page_range: Some("9-".into()), ..base() }).contains("not in this document"));
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 1, "refusals write nothing");
    }

    #[test]
    fn page_number_exports_are_byte_identical_across_runs() {
        let temp = TempDir::new().unwrap(); let input = temp.0.join("source.pdf"); page_text_fixture(&input);
        let scene = || { let mut s = text_scene(vec![page_number_object(PageNumberFormat::PageOfTotal, PageNumberPosition::BottomRight, 4, "all")]); s.pages.swap(0, 1); s };
        let first = export_text(&input, scene());
        let second = export_text(&input, scene());
        assert!(first.error().is_none() && second.error().is_none(), "{:?} {:?}", first.error(), second.error());
        assert_ne!(first.output_paths()[0], second.output_paths()[0]);
        assert_eq!(fs::read(&first.output_paths()[0]).unwrap(), fs::read(&second.output_paths()[0]).unwrap());
    }
}
