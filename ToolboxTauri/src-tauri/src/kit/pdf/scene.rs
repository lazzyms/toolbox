use std::{collections::{HashMap, HashSet}, fs::{self, OpenOptions}, io::Write, path::{Path, PathBuf}, process::Command, sync::atomic::{AtomicU64, Ordering}, time::{Instant}};
use base64::Engine;
use flate2::{write::ZlibEncoder, Compression};
use lopdf::{dictionary, Document, Object, ObjectId, Stream};
use quick_xml::{events::Event, escape::unescape, Reader as XmlReader, XmlVersion};
use serde::{Deserialize, Serialize};
use crate::kit::{common::{JobOutcome, OutputLocation, OutputNaming}, contracts::ToolError};
use super::metadata::{PdfDocumentMetadata, PdfPageMetadata, PdfTextRun};
use super::{inherited, resolve, PdfMutationIntent};

#[derive(Clone, Debug, Deserialize)]
pub struct Rect { pub x: f32, pub y: f32, pub width: f32, pub height: f32 }
#[derive(Clone, Debug, Deserialize)]
pub struct Point { pub x: f32, pub y: f32 }
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all="camelCase")]
pub struct SceneObject {
    pub kind: Kind, pub rect: Rect, pub text: String, pub font_size: f32,
    pub color: String, pub opacity: f32, pub strokes: Vec<Vec<Point>>,
    #[serde(default)] pub shape: Option<Shape>,
    #[serde(default)] pub highlight_mode: Option<HighlightMode>,
    #[serde(default)] pub signature_mode: Option<SignatureMode>,
    #[serde(default)] pub signature_path: Option<PathBuf>,
    #[serde(default)] pub font_family: Option<String>,
    #[serde(default)] pub watermark_pattern: Option<WatermarkPattern>,
}
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
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all="kebab-case")]
pub enum WatermarkPattern { AcrossPage, BottomRightToTopLeft, TopRightToBottomLeft, CenterHorizontal, CenterVertical }
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all="camelCase")]
pub struct ScenePage {
    pub source_index: Option<usize>, pub width: f32, pub height: f32,
    pub rotation: i32, pub crop: Option<Rect>,
    /// Flipped left to right before the turn, so the page displays as rotate(rotation) after the flip.
    #[serde(default)] pub mirrored: bool,
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
pub struct ExportRequest { pub paths: Vec<PathBuf>, pub scene: PdfScene, pub output_location: OutputLocation }
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct ScenePreview { pub data_url: String, pub width: u32, pub height: u32 }
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct IndexedScenePreview { pub page_index: usize, pub preview: ScenePreview }
#[derive(Deserialize)]
#[serde(rename_all="camelCase")]
pub struct SourceThumbnailsRequest { pub path: PathBuf, pub page_indices: Vec<usize> }
/// One requested source page: its thumbnail, or why it has none. A failed page never fails the others.
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct SourceThumbnail { pub page_index: usize, pub thumbnail: Option<ScenePreview>, pub error: Option<String> }

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

fn font_spec(value: Option<&str>) -> Result<(&'static str, &'static str), String> {
    match value.unwrap_or("Helvetica") {
        "Helvetica" => Ok(("Helvetica", "SceneFont")),
        "Helvetica-Bold" => Ok(("Helvetica-Bold", "SceneFontHelveticaBold")),
        "Helvetica-Oblique" => Ok(("Helvetica-Oblique", "SceneFontHelveticaOblique")),
        "Times-Roman" => Ok(("Times-Roman", "SceneFontTimesRoman")),
        "Times-Bold" => Ok(("Times-Bold", "SceneFontTimesBold")),
        "Times-Italic" => Ok(("Times-Italic", "SceneFontTimesItalic")),
        "Courier" => Ok(("Courier", "SceneFontCourier")),
        "Courier-Bold" => Ok(("Courier-Bold", "SceneFontCourierBold")),
        "Courier-Oblique" => Ok(("Courier-Oblique", "SceneFontCourierOblique")),
        _ => Err("Unsupported scene font".into()),
    }
}

const INCLUDED_SIGNATURE_SATISFY_FONT: &[u8] = include_bytes!("../../../resources/fonts/Satisfy-Regular.ttf");
const INCLUDED_SIGNATURE_PACIFICO_FONT: &[u8] = include_bytes!("../../../resources/fonts/Pacifico-Regular.ttf");
const SIGNATURE_RASTER_SCALE: f32 = 4.0;

fn signature_font_bytes(font_family: &str) -> Result<&'static [u8], String> {
    match font_family {
        "Satisfy" => Ok(INCLUDED_SIGNATURE_SATISFY_FONT),
        "Pacifico" => Ok(INCLUDED_SIGNATURE_PACIFICO_FONT),
        _ => Err("Unsupported cursive signature font".into()),
    }
}

fn encode_text(text: &str) -> Result<String, String> {
    if text.len() > 100_000 { return Err("Text is too long".into()); }
    let mut encoded = String::new();
    for c in text.chars() {
        if !((' '..='~').contains(&c) || ('\u{a0}'..='\u{ff}').contains(&c)) {
            return Err("Scene text currently supports Latin-1 characters only".into());
        }
        encoded.push_str(&format!("{:02X}", c as u32));
    }
    Ok(encoded)
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

fn watermark_placements(area: &Rect, anchor: &Rect, font_size: f32, text: &str, pattern: Option<&WatermarkPattern>) -> Vec<(f32, f32, f32)> {
    let pattern = pattern.unwrap_or(&WatermarkPattern::AcrossPage);
    let text_width = text.chars().count().max(1) as f32 * font_size * 0.6;
    let center_x = anchor.x + anchor.width / 2.;
    let center_y = anchor.y + anchor.height / 2.;
    match pattern {
        WatermarkPattern::AcrossPage => {
            let step_x = (text_width + font_size * 2.).max(100.);
            let step_y = (font_size * 4.).max(80.);
            let offset_x = anchor.x - (area.x + area.width * 0.14);
            let offset_y = anchor.y - (area.y + area.height * 0.33);
            let mut placements = Vec::new();
            let mut y = area.y - area.height + offset_y;
            while y <= area.y + area.height * 2. + offset_y {
                let offset = ((y - (area.y + offset_y)) / step_y).round() * step_x * 0.45;
                let mut x = area.x - area.width + offset_x + offset;
                while x <= area.x + area.width * 2. + offset_x {
                    placements.push((x, y, -35.));
                    x += step_x;
                }
                y += step_y;
            }
            placements
        }
        WatermarkPattern::BottomRightToTopLeft => vec![(center_x - text_width / 2., center_y + font_size / 2., -45.)],
        WatermarkPattern::TopRightToBottomLeft => vec![(center_x - text_width / 2., center_y + font_size / 2., 45.)],
        WatermarkPattern::CenterHorizontal => vec![(center_x - text_width / 2., center_y + font_size / 2., 0.)],
        WatermarkPattern::CenterVertical => vec![(center_x - text_width / 2., center_y + font_size / 2., 90.)],
    }
}

fn scene_angle_to_pdf_text_matrix(angle: f32) -> [f32; 4] {
    let radians = angle.to_radians();
    let (c, s) = (radians.cos(), radians.sin());
    [c, -s, -s, -c]
}

struct SignatureImage { rgb: Vec<u8>, alpha: Option<Vec<u8>>, width: u32, height: u32 }

fn compress_bytes(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes).map_err(err)?;
    encoder.finish().map_err(err)
}

fn signature_image(path: &Path) -> Result<SignatureImage, String> {
    let image = image::open(path).map_err(|error| format!("Could not read signature image: {error}"))?.to_rgba8();
    if image.width() == 0 || image.height() == 0 || (image.width() as u64) * (image.height() as u64) > 20_000_000 {
        return Err("Signature image dimensions are not supported".into());
    }
    let mut rgb = Vec::with_capacity((image.width() * image.height() * 3) as usize);
    let mut alpha = Vec::with_capacity((image.width() * image.height()) as usize);
    for pixel in image.pixels() {
        rgb.extend_from_slice(&pixel.0[..3]);
        alpha.push(pixel.0[3]);
    }
    let alpha = alpha.iter().any(|value| *value < 255).then_some(alpha);
    Ok(SignatureImage { rgb: compress_bytes(&rgb)?, alpha: alpha.map(|values| compress_bytes(&values)).transpose()?, width: image.width(), height: image.height() })
}

fn signature_text_image(text: &str, font_family: &str, font_size: f32, color: [f32; 3]) -> Result<SignatureImage, String> {
    if text.trim().is_empty() { return Err("Signature text is required when using a typed signature".into()); }
    if text.len() > 100_000 { return Err("Text is too long".into()); }
    let bytes = signature_font_bytes(font_family)?;
    let font = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()).map_err(|error| format!("Could not parse signature font: {error}"))?;
    let raster_size = (font_size * SIGNATURE_RASTER_SCALE).clamp(24.0, 512.0);
    let line_metrics = font.horizontal_line_metrics(raster_size).ok_or("Could not measure signature font")?;
    let line_height = (line_metrics.ascent - line_metrics.descent).max(raster_size * 1.1);
    let lines = text.split('\n').collect::<Vec<_>>();
    let mut line_widths = Vec::with_capacity(lines.len());
    for line in &lines {
        let mut width = 0.0;
        for character in line.chars() {
            if !font.has_glyph(character) { return Err(format!("Signature font does not support character {character:?}")); }
            width += font.metrics(character, raster_size).advance_width;
        }
        line_widths.push(width);
    }
    let width = (line_widths.iter().copied().fold(0.0, f32::max) + 16.0).ceil() as usize;
    let height = (line_height * lines.len() as f32 + 16.0).ceil() as usize;
    let pixels = width.checked_mul(height).ok_or("Signature image dimensions overflowed")?;
    if width == 0 || height == 0 || pixels > 20_000_000 { return Err("Signature text dimensions are not supported".into()); }
    let color = color.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8);
    let mut alpha = vec![0_u8; pixels];
    for (line_index, line) in lines.iter().enumerate() {
        let baseline = 8.0 + line_index as f32 * line_height + line_metrics.ascent;
        let mut pen_x = 8.0;
        for character in line.chars() {
            let metrics = font.metrics(character, raster_size);
            let (_, bitmap) = font.rasterize(character, raster_size);
            let glyph_x = (pen_x + metrics.bounds.xmin).floor() as i64;
            let glyph_y = (baseline - metrics.bounds.height - metrics.bounds.ymin).floor() as i64;
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
    let alpha = alpha.iter().any(|value| *value < 255).then_some(alpha);
    Ok(SignatureImage { rgb: compress_bytes(&rgb)?, alpha: alpha.map(|values| compress_bytes(&values)).transpose()?, width: width as u32, height: height as u32 })
}

fn add_signature_image(document: &mut Document, xobjects: &mut lopdf::Dictionary, name: &str, image: SignatureImage) {
    let mask = image.alpha.map(|alpha| document.add_object(Object::Stream(Stream::new(dictionary! {
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
    let id = document.add_object(Object::Stream(Stream::new(properties, image.rgb)));
    xobjects.set(name, id);
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
    let annotation_owners = if scene.pages.iter().any(|page| page.mirrored) { annotation_owner_counts(document, source_pages)? } else { HashMap::new() };
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
            // A mirror alone is remapped: annotations, widgets and destinations follow the flip. A crop or
            // resize still cannot move them safely, so it is refused as before.
            let crops_page = page_box_changed(page, &geometry);
            navigation_geometry_edit |= crops_page;
            if has_annotations && crops_page {
                return Err("This PDF page has annotations or form widgets and the requested crop would change their coordinates; scene export was rejected before output".into());
            }
            if (page.crop.is_some() || crops_page) && has_widget {
                return Err("Cropping a PDF with form widgets is unsupported; remove the crop or edit the source PDF first".into());
            }
            if page.mirrored && has_annotations && page_annotation_ids(document, page_id)?.iter().any(|id| annotation_owners.get(id).copied().unwrap_or(0) > 1) {
                return Err("This PDF page shares annotations or form widgets with another page, so mirroring it would move them on both; scene export was rejected before output".into());
            }
        }
    }
    if has_navigation && navigation_geometry_edit {
        return Err("This PDF contains bookmarks or destinations whose coordinates cannot be transformed safely after a crop; scene export was rejected before output".into());
    }
    Ok(ScenePreflight { pages_root, source_pages: source_pages.to_vec() })
}

fn same_box(left: [f32; 4], right: [f32; 4]) -> bool { left.iter().zip(right).all(|(left, right)| (*left - right).abs() <= 0.1) }
fn same_rect(left: &Rect, right: &Rect) -> bool { same_box([left.x, left.y, left.x + left.width, left.y + left.height], [right.x, right.y, right.x + right.width, right.y + right.height]) }
/// Whether the scene crops, resizes or mirrors the page, which moves its content in page space.
/// A mirror redraws the page through a flipped Form XObject, the same path a crop takes, so it stays vector.
/// Rotation is not a geometry change: it is added to the page's own /Rotate entry, which turns
/// annotations, form widgets and destinations with the page and leaves their coordinates alone.
fn page_geometry_changed(page: &ScenePage, geometry: &Geometry) -> bool {
    page.mirrored || page_box_changed(page, geometry)
}
/// Whether the scene crops or resizes the page. Unlike a mirror, this has no transform that keeps
/// annotations, widgets and destinations on the content they point at.
fn page_box_changed(page: &ScenePage, geometry: &Geometry) -> bool {
    let full_page = Rect { x: 0., y: 0., width: page.width, height: page.height };
    page.crop.as_ref().is_some_and(|crop| !same_rect(crop, &full_page))
        || (page.width - geometry.width).abs() > 0.1
        || (page.height - geometry.height).abs() > 0.1
}

/// The annotation references a page lists, plus its /Annots array when that array is its own object.
fn page_annotation_ids(document: &Document, page: ObjectId) -> Result<Vec<ObjectId>, String> {
    let Some(value) = document.get_dictionary(page).map_err(err)?.get(b"Annots").ok() else { return Ok(vec![]); };
    let mut ids = value.as_reference().ok().into_iter().collect::<Vec<_>>();
    ids.extend(resolve(document, value)?.as_array().map_err(err)?.iter().filter_map(|annotation| annotation.as_reference().ok()));
    Ok(ids)
}
/// How many times each annotation (or /Annots array) is listed across the source pages.
fn annotation_owner_counts(document: &Document, pages: &[ObjectId]) -> Result<HashMap<ObjectId, usize>, String> {
    let mut counts = HashMap::new();
    for page in pages { for id in page_annotation_ids(document, *page)? { *counts.entry(id).or_insert(0) += 1; } }
    Ok(counts)
}

/// Matrix `outer · inner`: maps a point through `inner`, then through `outer`.
fn concat(outer: [f32; 6], inner: [f32; 6]) -> [f32; 6] {
    let [a, b, c, d, e, f] = outer;
    let [ia, ib, ic, id, ie, iff] = inner;
    [a * ia + c * ib, b * ia + d * ib, a * ic + c * id, b * ic + d * id, a * ie + c * iff + e, b * ie + d * iff + f]
}
fn apply(m: [f32; 6], (x, y): (f32, f32)) -> (f32, f32) { (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]) }
/// The matrix from a mirrored source page's user space to the user space of the page it is redrawn on.
/// It is the same chain the redrawn content takes in `compose`: the source turn and box offset undone,
/// the flip around the visible area, then back to PDF's bottom-up axis.
fn mirrored_page_matrix(geometry: &Geometry, page: &ScenePage) -> [f32; 6] {
    let crop = page.crop.clone().unwrap_or(Rect { x: 0., y: 0., width: page.width, height: page.height });
    let flip = [-1., 0., 0., 1., 2. * crop.x + crop.width, 0.];
    let outer = [1., 0., 0., -1., -crop.x, crop.y + crop.height];
    concat(outer, concat(flip, geometry.matrix))
}
/// The matrix only ever swaps or negates axes, so a coordinate maps from exactly one source axis.
fn swaps_axes(m: [f32; 6]) -> bool { m[0].abs() < 0.5 }
fn mapped_x(m: [f32; 6], x: Option<f32>, y: Option<f32>) -> Option<f32> { if swaps_axes(m) { y.map(|y| m[2] * y + m[4]) } else { x.map(|x| m[0] * x + m[4]) } }
fn mapped_y(m: [f32; 6], x: Option<f32>, y: Option<f32>) -> Option<f32> { if swaps_axes(m) { x.map(|x| m[1] * x + m[5]) } else { y.map(|y| m[3] * y + m[5]) } }

fn coordinate(document: &Document, value: &Object) -> Result<Option<f32>, String> {
    match resolve(document, value)? {
        Object::Null => Ok(None),
        value => super::metadata::number(value).map(Some).map_err(|_| "A destination or annotation coordinate is not a number; scene export was rejected before output".to_string()),
    }
}
fn numbers(document: &Document, value: &Object) -> Result<Vec<f32>, String> {
    resolve(document, value)?.as_array().map_err(err)?.iter()
        .map(|value| coordinate(document, value)?.filter(|value| value.is_finite()).ok_or_else(|| "An annotation coordinate is missing; scene export was rejected before output".to_string()))
        .collect()
}
fn optional(value: Option<f32>) -> Object { value.map_or(Object::Null, Object::Real) }
fn mapped_rect(m: [f32; 6], rect: &[f32]) -> [f32; 4] {
    let (x1, y1) = apply(m, (rect[0], rect[1]));
    let (x2, y2) = apply(m, (rect[2], rect[3]));
    [x1.min(x2), y1.min(y2), x1.max(x2), y1.max(y2)]
}
fn mapped_points(document: &Document, value: &Object, m: [f32; 6]) -> Result<Object, String> {
    let values = numbers(document, value)?;
    if values.len() % 2 != 0 { return Err("An annotation point list has an odd number of coordinates; scene export was rejected before output".into()); }
    Ok(array(&values.chunks(2).flat_map(|point| { let (x, y) = apply(m, (point[0], point[1])); [x, y] }).collect::<Vec<_>>()))
}
/// A copy of an appearance stream whose /Matrix also applies the page's flip and turn. A viewer fits
/// the transformed /BBox to the annotation's /Rect, so only the linear part is needed.
fn mapped_appearance(document: &mut Document, value: &Object, linear: [f32; 6]) -> Result<Object, String> {
    match resolve(document, value)?.clone() {
        Object::Stream(mut stream) => {
            let matrix = match stream.dict.get(b"Matrix").ok() {
                Some(matrix) => numbers(document, matrix)?.try_into().map_err(|_| "An appearance stream has an invalid /Matrix; scene export was rejected before output".to_string())?,
                None => [1., 0., 0., 1., 0., 0.],
            };
            stream.dict.set("Matrix", array(&concat(linear, matrix)));
            Ok(Object::Reference(document.add_object(stream)))
        }
        Object::Dictionary(states) => {
            let mut mapped = lopdf::Dictionary::new();
            for (state, appearance) in states.iter() { mapped.set(state.clone(), mapped_appearance(document, appearance, linear)?); }
            Ok(Object::Dictionary(mapped))
        }
        _ => Err("An annotation appearance is not a stream; scene export was rejected before output".into()),
    }
}
/// Moves one annotation of a mirrored page onto the redrawn page: its rectangle, points, insets and
/// appearance follow the content it covers. Appearance streams are copied, so a shared one is untouched.
fn mirror_annotation(document: &mut Document, annotation: &mut lopdf::Dictionary, m: [f32; 6], source_rotation: i64) -> Result<(), String> {
    let linear = [m[0], m[1], m[2], m[3], 0., 0.];
    if let Ok(value) = annotation.get(b"Rect") {
        let rect = numbers(document, value)?;
        if rect.len() != 4 { return Err("An annotation rectangle is invalid; scene export was rejected before output".into()); }
        let mapped = mapped_rect(m, &rect);
        if let Ok(value) = annotation.get(b"RD") {
            let insets = numbers(document, value)?;
            if insets.len() != 4 { return Err("An annotation inset is invalid; scene export was rejected before output".into()); }
            let (left, top, right, bottom) = (rect[0].min(rect[2]), rect[1].max(rect[3]), rect[0].max(rect[2]), rect[1].min(rect[3]));
            let inner = mapped_rect(m, &[left + insets[0], bottom + insets[3], right - insets[2], top - insets[1]]);
            annotation.set("RD", array(&[inner[0] - mapped[0], mapped[3] - inner[3], mapped[2] - inner[2], inner[1] - mapped[1]]));
        }
        annotation.set("Rect", array(&mapped));
    }
    for key in [b"QuadPoints".as_slice(), b"Vertices", b"L", b"CL"] {
        if let Ok(value) = annotation.get(key) { let mapped = mapped_points(document, &value.clone(), m)?; annotation.set(key.to_vec(), mapped); }
    }
    if let Ok(value) = annotation.get(b"InkList") {
        let strokes = resolve(document, value)?.as_array().map_err(err)?.clone();
        let mapped = strokes.iter().map(|stroke| mapped_points(document, stroke, m)).collect::<Result<Vec<_>, _>>()?;
        annotation.set("InkList", mapped);
    }
    if let Ok(value) = annotation.get(b"AP") {
        let appearances = resolve(document, value)?.as_dict().map_err(err)?.clone();
        let mut mapped = lopdf::Dictionary::new();
        for (kind, appearance) in appearances.iter() { mapped.set(kind.clone(), mapped_appearance(document, appearance, linear)?); }
        annotation.set("AP", mapped);
    }
    // The source turn is drawn into the redrawn page, so a widget's own turn no longer needs to undo it.
    let is_widget = annotation.get(b"Subtype").ok().and_then(|value| value.as_name().ok()) == Some(b"Widget");
    if is_widget && source_rotation != 0 {
        let mut characteristics = match annotation.get(b"MK").ok() { Some(value) => resolve(document, value)?.as_dict().map_err(err)?.clone(), None => lopdf::Dictionary::new() };
        let turn = match characteristics.get(b"R").ok() { Some(value) => resolve(document, value)?.as_i64().map_err(err)?, None => 0 };
        characteristics.set("R", (turn - source_rotation).rem_euclid(360));
        annotation.set("MK", characteristics);
    }
    Ok(())
}
fn mirror_page_annotations(document: &mut Document, page: ObjectId, m: [f32; 6], source_rotation: i64) -> Result<(), String> {
    let Some(value) = document.get_dictionary(page).map_err(err)?.get(b"Annots").ok().cloned() else { return Ok(()); };
    let mut annotations = resolve(document, &value)?.as_array().map_err(err)?.clone();
    for entry in annotations.iter_mut() {
        match entry {
            Object::Reference(id) => {
                let id = *id;
                let mut annotation = document.get_dictionary(id).map_err(err)?.clone();
                mirror_annotation(document, &mut annotation, m, source_rotation)?;
                document.objects.insert(id, Object::Dictionary(annotation));
            }
            Object::Dictionary(annotation) => mirror_annotation(document, annotation, m, source_rotation)?,
            _ => return Err("A page annotation is not a dictionary; scene export was rejected before output".into()),
        }
    }
    match value {
        Object::Reference(id) => { document.objects.insert(id, Object::Array(annotations)); }
        _ => { document.get_dictionary_mut(page).map_err(err)?.set("Annots", annotations); }
    }
    Ok(())
}
/// An explicit destination to a mirrored page, mapped so it lands on the same content. A fit to one
/// edge keeps pointing at that edge, so on a page whose source turn swaps axes a /FitH becomes a /FitV.
fn mirror_destination(document: &Document, destination: &mut [Object], m: [f32; 6]) -> Result<(), String> {
    let kind = destination[1].as_name().map_err(err)?.to_vec();
    let at = |index: usize| destination.get(index).map(|value| coordinate(document, value)).transpose().map(Option::flatten);
    let swapped = swaps_axes(m);
    let edge = |name: &[u8], value: Option<f32>, horizontal: bool| -> (Vec<u8>, Object) {
        // `horizontal` fits pin a top (y); the others pin a left (x).
        let (x, y) = if horizontal { (None, value) } else { (value, None) };
        let horizontal_after = horizontal != swapped;
        let mapped = if horizontal_after { mapped_y(m, x, y) } else { mapped_x(m, x, y) };
        let bounded = name.starts_with(b"FitB");
        let name = match (bounded, horizontal_after) { (false, true) => b"FitH".to_vec(), (false, false) => b"FitV".to_vec(), (true, true) => b"FitBH".to_vec(), (true, false) => b"FitBV".to_vec() };
        (name, optional(mapped))
    };
    match kind.as_slice() {
        b"XYZ" => {
            let (left, top) = (at(2)?, at(3)?);
            if destination.len() > 2 { destination[2] = optional(mapped_x(m, left, top)); }
            if destination.len() > 3 { destination[3] = optional(mapped_y(m, left, top)); }
        }
        b"FitH" | b"FitBH" | b"FitV" | b"FitBV" => {
            let horizontal = kind.ends_with(b"H");
            let (name, value) = edge(&kind, at(2)?, horizontal);
            destination[1] = Object::Name(name);
            if destination.len() > 2 { destination[2] = value; }
        }
        b"FitR" => {
            let values = (2..6).map(at).collect::<Result<Option<Vec<_>>, _>>()?.ok_or("A /FitR destination is missing a coordinate; scene export was rejected before output")?;
            let mapped = mapped_rect(m, &values);
            for (index, value) in mapped.into_iter().enumerate() { destination[2 + index] = Object::Real(value); }
        }
        _ => {}
    }
    Ok(())
}
fn is_destination_to(value: &[Object], pages: &HashMap<ObjectId, [f32; 6]>) -> Option<[f32; 6]> {
    let page = value.first()?.as_reference().ok()?;
    let kind = value.get(1)?.as_name().ok()?;
    let known = matches!(kind, b"XYZ" | b"Fit" | b"FitH" | b"FitV" | b"FitR" | b"FitB" | b"FitBH" | b"FitBV");
    if known { pages.get(&page).copied() } else { None }
}
fn mirror_destinations_in(document: &Document, value: &mut Object, pages: &HashMap<ObjectId, [f32; 6]>) -> Result<(), String> {
    match value {
        Object::Array(items) => {
            if let Some(m) = is_destination_to(items, pages) { return mirror_destination(document, items, m); }
            for item in items.iter_mut() { mirror_destinations_in(document, item, pages)?; }
        }
        Object::Dictionary(dictionary) => for (_, item) in dictionary.iter_mut() { mirror_destinations_in(document, item, pages)?; },
        _ => {}
    }
    Ok(())
}
/// Maps every explicit destination that targets a mirrored page, wherever it is stored: bookmarks,
/// named destinations, link and widget actions, the open action. Each object is visited once.
fn mirror_destinations(document: &mut Document, pages: &HashMap<ObjectId, [f32; 6]>) -> Result<(), String> {
    let ids = document.objects.keys().copied().collect::<Vec<_>>();
    for id in ids {
        let object = document.objects.get(&id).ok_or("A PDF object disappeared during scene export")?;
        // Only a stream's dictionary can hold a destination, so its content is never copied.
        let mut value = match object { Object::Stream(stream) => Object::Dictionary(stream.dict.clone()), object => object.clone() };
        mirror_destinations_in(document, &mut value, pages)?;
        match (document.objects.get_mut(&id), value) {
            (Some(Object::Stream(stream)), Object::Dictionary(dict)) => stream.dict = dict,
            (_, value) => { document.objects.insert(id, value); }
        }
    }
    Ok(())
}
/// The /Rotate a kept source page gets: its source rotation plus the scene's quarter turns.
fn effective_rotation(document: &Document, page_id: ObjectId, page: &ScenePage) -> Result<i64, String> {
    if page.rotation % 90 != 0 { return Err("Rotation must be a multiple of 90".into()); }
    let source = inherited(document, page_id, b"Rotate")?.map(|value| value.as_i64().map_err(err)).transpose()?.unwrap_or(0);
    Ok((source + i64::from(page.rotation)).rem_euclid(360))
}
fn rotate_kept_page(document: &mut Document, page_id: ObjectId, page: &ScenePage) -> Result<(), String> {
    if page.rotation.rem_euclid(360) == 0 { return Ok(()); }
    let rotation = effective_rotation(document, page_id, page)?;
    document.get_dictionary_mut(page_id).map_err(err)?.set("Rotate", rotation);
    Ok(())
}
fn inverse_matrix([a, b, c, d, e, f]: [f32; 6]) -> [f32; 6] {
    let determinant = a * d - b * c;
    [d / determinant, -b / determinant, -c / determinant, a / determinant,
        (c * f - d * e) / determinant, (b * e - a * f) / determinant]
        .map(|value| if value == 0. { 0. } else { value })
}
fn append_contents(original: Option<Object>, overlay: Object) -> Object {
    match original {
        Some(Object::Array(mut contents)) => { contents.push(overlay); Object::Array(contents) }
        Some(contents) => Object::Array(vec![contents, overlay]),
        None => overlay,
    }
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
        if page.source_index != Some(index) || !page.objects.is_empty() || page.rotation.rem_euclid(360) != 0 || page.mirrored { return Ok(false); }
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

pub fn compose(path:&Path, scene:&PdfScene)->Result<Document,String> {
    if scene.pages.is_empty() || scene.pages.len()>2000 { return Err("Scene must contain 1–2000 pages".into()); }
    let mut doc=load(path)?;
    let sources:Vec<_>=doc.get_pages().values().copied().collect();
    let identity = is_identity_scene(&doc, scene, &sources)?;
    let pure_reorder = is_pure_page_reorder(&doc, scene, &sources)?;
    let preflight = scene_preflight(&doc, scene, &sources, identity, pure_reorder)?;
    if identity { return Ok(doc); }
    if pure_reorder {
        let order = scene.pages.iter().map(|page| {
            let index = page.source_index.ok_or_else(|| "Pure page reorder is missing a source page".to_string())?;
            sources.get(index).copied().ok_or_else(|| "Scene source page is outside the document".to_string())
        }).collect::<Result<Vec<_>, String>>()?;
        for (page, id) in scene.pages.iter().zip(&order) { rotate_kept_page(&mut doc, *id, page)?; }
        let root = doc.get_dictionary_mut(preflight.pages_root).map_err(err)?;
        root.set("Kids", order.iter().map(|page| Object::Reference(*page)).collect::<Vec<_>>());
        root.set("Count", order.len() as i64);
        return Ok(doc);
    }
    // Mirrored pages keep their object ids, so their annotations and every destination that targets
    // them are moved to the mirrored position once, from the source geometry, before any page is redrawn.
    let mut mirrored_pages = HashMap::new();
    for page in scene.pages.iter().filter(|page| page.mirrored) {
        let Some(index) = page.source_index else { continue; };
        let id = *sources.get(index).ok_or("Source page index outside document")?;
        let matrix = mirrored_page_matrix(&geometry(&doc, id)?, page);
        let source_rotation = super::metadata::page_rotation(&doc, id);
        mirror_page_annotations(&mut doc, id, matrix, source_rotation)?;
        mirrored_pages.insert(id, matrix);
    }
    if !mirrored_pages.is_empty() { mirror_destinations(&mut doc, &mirrored_pages)?; }
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
    let mut kids=Vec::new();
    for page in &scene.pages {
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
        if preserve_source_page && page.objects.is_empty() {
            let id = sources[page.source_index.unwrap()];
            rotate_kept_page(&mut doc, id, page)?;
            kids.push(Object::Reference(id));
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
        let mut effective_xobjects = source_index.map(|index| effective_resource_entries(&doc, sources[index], b"XObject")).transpose()?.unwrap_or_default();
        let mut effective_states = source_index.map(|index| effective_resource_entries(&doc, sources[index], b"ExtGState")).transpose()?.unwrap_or_default();
        let mut xobjects=lopdf::Dictionary::new();
        let mut content=if preserve_source_page {
            let g = source_geometry.as_ref().unwrap();
            format!("q\n{}0 0 {} {} re W n\n", cm(inverse_matrix(g.matrix)), page.width, page.height)
        } else {
            format!("q\n1 0 0 -1 {} {} cm\n0 0 {} {} re W n\n",-crop.x,crop.y+crop.height,page.width,page.height)
        };
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
                // A mirror flips the source left to right around the visible area, in scene coordinates,
                // before the page's turn. Only the original's content flips: scene objects draw as placed.
                let flip = if page.mirrored { cm([-1., 0., 0., 1., 2. * crop.x + crop.width, 0.]) } else { String::new() };
                content.push_str(&format!("q\n{flip}{}\n/{original_name} Do\nQ\n", cm(g.matrix)));
            }
        }
        if page.objects.len()>10000 { return Err("Too many scene objects".into()); }
        let mut states=lopdf::Dictionary::new();
        for (i,o) in page.objects.iter().enumerate() {
            valid_rect(&o.rect)?;
            if !o.opacity.is_finite() || !(0. ..=1.).contains(&o.opacity) { return Err("Opacity must be between 0 and 1".into()); }
            if !o.font_size.is_finite() || o.font_size<=0. || o.font_size>1000. { return Err("Invalid font size".into()); }
            if o.highlight_mode.is_some() && !matches!(&o.kind, Kind::Highlight) { return Err("Highlight mode is only valid for highlight objects".into()); }
            let rgb=rgb(&o.color)?;
            let state_base = format!("S{i}");
            let state_name = next_resource_name(&effective_states, "ExtGState", &state_base)?;
            effective_states.set(state_name.as_str(), Object::Null);
            states.set(state_name.as_str(),dictionary!{"Type"=>"ExtGState","ca"=>o.opacity,"CA"=>o.opacity});
            let r=&o.rect;
            content.push_str(&format!("q /{state_name} gs {} {} {} rg {} {} {} RG 2 w 1 J 1 j\n",rgb[0],rgb[1],rgb[2],rgb[0],rgb[1],rgb[2]));
            match o.kind {
                Kind::Highlight=>content.push_str(&format!("{} {} {} {} re f\n",r.x,r.y,r.width,r.height)),
                Kind::Shape=>content.push_str(&shape_content(o.shape.as_ref(), r)),
                Kind::Signature=>{
                    match o.signature_mode {
                        Some(SignatureMode::Image) => {
                            let path=o.signature_path.as_ref().ok_or("Choose a signature image before placing it")?;
                            let name_base = format!("Sig{i}");
                            let name = next_resource_name(&effective_xobjects, "XObject", &name_base)?;
                            effective_xobjects.set(name.as_str(), Object::Null);
                            add_signature_image(&mut doc, &mut xobjects, &name, signature_image(path)?);
                            content.push_str(&format!("q {} 0 0 {} {} {} /{name} Do Q\n",r.width,r.height,r.x,r.y));
                        }
                        Some(SignatureMode::Text) => {
                            if let Some(font_family) = o.font_family.as_deref().filter(|font| matches!(*font, "Satisfy" | "Pacifico")) {
                                let name_base = format!("Sig{i}");
                                let name = next_resource_name(&effective_xobjects, "XObject", &name_base)?;
                                effective_xobjects.set(name.as_str(), Object::Null);
                                add_signature_image(&mut doc, &mut xobjects, &name, signature_text_image(&o.text, font_family, o.font_size, rgb)?);
                                content.push_str(&format!("q {} 0 0 {} {} {} /{name} Do Q\n",r.width,r.height,r.x,r.y));
                            } else {
                                let (_, resource)=font_spec(o.font_family.as_deref())?;
                                let resource = font_names.get(resource).ok_or("Scene font resource disappeared")?;
                                let encoded=encode_text(&o.text)?;
                                content.push_str(&format!("BT /{resource} {} Tf 1 0 0 -1 {} {} Tm <{encoded}> Tj ET\n",o.font_size,r.x,r.y+o.font_size));
                            }
                        }
                        None => {
                            if o.strokes.iter().map(Vec::len).sum::<usize>()>100000 { return Err("Signature is too large".into()); }
                            for stroke in &o.strokes {
                                for (j,p) in stroke.iter().enumerate() {
                                    if !p.x.is_finite() || !p.y.is_finite() || !(0. ..=1.).contains(&p.x) || !(0. ..=1.).contains(&p.y) { return Err("Invalid signature point".into()); }
                                    content.push_str(&format!("{} {} {}\n",r.x+p.x*r.width,r.y+p.y*r.height,if j==0 {"m"} else {"l"}));
                                }
                                if stroke.len()==1 { let p=&stroke[0]; content.push_str(&format!("{} {} l\n",r.x+p.x*r.width,r.y+p.y*r.height)); }
                                content.push_str("S\n");
                            }
                        }
                    }
                },
                Kind::Text=>{
                    let (_, resource)=font_spec(o.font_family.as_deref())?;
                    let resource = font_names.get(resource).ok_or("Scene font resource disappeared")?;
                    let encoded_lines=o.text.lines().map(encode_text).collect::<Result<Vec<_>,_>>()?;
                    content.push_str(&format!("{} {} {} {} re W n\n",r.x,r.y,r.width,r.height));
                    for (line,encoded) in encoded_lines.iter().enumerate() {
                        content.push_str(&format!("BT /{resource} {} Tf 1 0 0 -1 {} {} Tm <{}> Tj ET\n",o.font_size,r.x,r.y+o.font_size*(1.+line as f32*1.2),encoded));
                    }
                },
                Kind::Watermark=>{
                    let (_, resource)=font_spec(o.font_family.as_deref())?;
                    let resource = font_names.get(resource).ok_or("Scene font resource disappeared")?;
                    let encoded=encode_text(&o.text)?;
                    let area=page.crop.as_ref().cloned().unwrap_or(Rect{x:0.,y:0.,width:page.width,height:page.height});
                    for (x,y,angle) in watermark_placements(&area,&o.rect,o.font_size,&o.text,o.watermark_pattern.as_ref()) {
                        let [a, b, c, d] = scene_angle_to_pdf_text_matrix(angle);
                        content.push_str(&format!("BT /{resource} {} Tf {} {} {} {} {} {} Tm <{encoded}> Tj ET\n",o.font_size,a,b,c,d,x,y));
                    }
                },
            }
            content.push_str("Q\n");
        }
        content.push_str("Q\n");
        let stream=doc.add_object(Stream::new(dictionary!{},content.into_bytes()));
        if let Some(index) = source_index {
            let id = preflight.source_pages[index];
            let resources = scene_resources(&doc, id, xobjects, font_resources, states)?;
            if preserve_source_page {
                let page_dict = doc.get_dictionary_mut(id).map_err(err)?;
                let original_contents = page_dict.get(b"Contents").ok().cloned();
                page_dict.set("Resources",resources);
                page_dict.set("Contents",append_contents(original_contents,Object::Reference(stream)));
                rotate_kept_page(&mut doc, id, page)?;
            } else {
                super::set_page_box_family(&mut doc, id, [0., 0., crop.width, crop.height], false)?;
                let page_dict = doc.get_dictionary_mut(id).map_err(err)?;
                page_dict.set("Rotate",page.rotation.rem_euclid(360));
                page_dict.set("Resources",resources);
                page_dict.set("Contents",stream);
            }
            kids.push(Object::Reference(id));
        } else {
            let id=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>preflight.pages_root,"MediaBox"=>array(&[0.,0.,crop.width,crop.height]),"Rotate"=>page.rotation.rem_euclid(360),"Resources"=>dictionary!{"XObject"=>xobjects,"Font"=>font_resources,"ExtGState"=>states},"Contents"=>stream});
            kids.push(Object::Reference(id));
        }
    }
    doc.objects.get_mut(&preflight.pages_root).ok_or("PDF page-tree root disappeared during scene export")?.as_dict_mut().map_err(err)?.set("Kids",kids);
    doc.objects.get_mut(&preflight.pages_root).ok_or("PDF page-tree root disappeared during scene export")?.as_dict_mut().map_err(err)?.set("Count",scene.pages.len() as i64);
    Ok(doc)
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
    render_at(path,index,renderer,temp,super::PDF_PREVIEW_MAX_DIMENSION,Instant::now() + super::PDF_PREVIEW_TIMEOUT)
}
fn render_at(path:&Path,index:usize,renderer:&Path,temp:&TempDir,max_dimension:u32,deadline:Instant)->Result<ScenePreview,String> {
    // A per-page name, so a page whose helper writes nothing can never pick up an earlier page's image.
    let prefix=temp.0.join(format!("render-{index}"));
    let output=super::run_bounded_helper({
        let mut command = Command::new(renderer);
        command.args(["-png","-singlefile","-scale-to"]).arg(max_dimension.to_string()).arg("-f").arg((index+1).to_string()).arg("-l").arg((index+1).to_string()).arg(path).arg(&prefix);
        command
    }, deadline, 64 * 1024, super::PDF_TEXT_MAX_ERROR_BYTES, "PDF preview renderer timed out.").map_err(err)?;
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
/// The latest a helper run may finish when it renders `pages` pages of one file: the per-file budget plus the per-page budget for each page.
fn thumbnail_deadline(start:Instant,pages:u32)->Instant {
    start + super::PDF_THUMBNAIL_FILE_TIMEOUT + super::PDF_THUMBNAIL_PAGE_TIMEOUT * pages
}
fn validate_thumbnail_page_indices(page_indices:&[usize], page_count:usize)->Result<(),String> {
    if !(1..=super::PDF_THUMBNAIL_MAX_BATCH).contains(&page_indices.len()) { return Err(format!("Thumbnail requests must include 1–{} page indices", super::PDF_THUMBNAIL_MAX_BATCH)); }
    let mut seen=HashSet::with_capacity(page_indices.len());
    for &index in page_indices {
        if index>=page_count { return Err("Thumbnail page outside document".into()); }
        if !seen.insert(index) { return Err("Thumbnail page indices must be unique".into()); }
    }
    Ok(())
}
/// Low-resolution thumbnails of the original pages, not the edited scene, for the Organize grid.
/// Each page renders in its own helper run under its own deadline, so one bad page only loses its own thumbnail.
pub fn source_thumbnails(request:&SourceThumbnailsRequest)->Result<Vec<SourceThumbnail>,ToolError> {
    let page_count=Document::load_metadata(&request.path).map_err(|_| ToolError::invalid_input(UNREADABLE_PDF_MESSAGE))?.page_count as usize;
    validate_thumbnail_page_indices(&request.page_indices,page_count).map_err(ToolError::invalid_input)?;
    let renderer=renderer().map_err(ToolError::unavailable)?;
    let temp=TempDir::new().map_err(ToolError::processing)?;
    Ok(request.page_indices.iter().copied().map(|page_index| {
        match render_at(&request.path,page_index,&renderer,&temp,super::PDF_THUMBNAIL_MAX_DIMENSION,thumbnail_deadline(Instant::now(),1)) {
            Ok(thumbnail)=>SourceThumbnail{page_index,thumbnail:Some(thumbnail),error:None},
            Err(error)=>SourceThumbnail{page_index,thumbnail:None,error:Some(error)},
        }
    }).collect())
}
pub const LOCKED_PDF_MESSAGE:&str="This PDF is password-protected. Remove its password with File Security first, then open the unlocked copy.";
pub const UNREADABLE_PDF_MESSAGE:&str="Toolbox can't read this file. It may be damaged, or it isn't a PDF that Toolbox supports. Nothing was changed.";

/// Refuses a file over the recorded scene-export limits before any page is read, so the editors never show
/// a partial grid for a document they could not export. A page-count refusal says how many pages it has and points to Split.
fn check_scene_limits(path:&Path)->Result<(),ToolError> {
    let limits=crate::kit::common::limits::recorded("export_pdf_scene").expect("scene export records its limits");
    let error=match limits.check(path) { Ok(())=>return Ok(()), Err(error)=>error };
    if !matches!(error.kind,crate::kit::contracts::ErrorKind::LimitExceeded{limit:crate::kit::contracts::LimitKind::PageCount}) { return Err(error); }
    let Some(max_pages)=limits.max_pages else { return Err(error) };
    let label=crate::kit::common::limits::count_label;
    let pages=Document::load_metadata(path).map(|metadata| label(u64::from(metadata.page_count))).unwrap_or_else(|_| format!("more than {}", label(u64::from(max_pages))));
    Err(ToolError::limit_exceeded(crate::kit::contracts::LimitKind::PageCount,
        format!("It has {pages} pages. Toolbox works with up to {} pages in one PDF. Use Split to make smaller parts, then open each one.", label(u64::from(max_pages))))
        .with_pointer(crate::kit::contracts::OutcomePointer::Tool{tool_id:"pdf-split".into()}))
}

/// Reads page geometry for the scene editors. Password-protected and unreadable files come back as typed
/// invalid-input errors so the UI can explain them (and point locked files to File Security). Files over the
/// recorded limits come back as LimitExceeded before the document is loaded.
pub fn inspect(path:&Path)->Result<PdfDocumentMetadata,ToolError> {
    check_scene_limits(path)?;
    let locked=|| ToolError::invalid_input(LOCKED_PDF_MESSAGE).with_pointer(crate::kit::contracts::OutcomePointer::Tool{tool_id:"lock-unlock".into()});
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
        pages.push(PdfPageMetadata{index,x:g.bbox[0],y:g.bbox[1],width:g.width,height:g.height,rotation:source_rotation,page_box:g.bbox,preview:None,text_runs:None});
    }
    if pages.is_empty() { return Err(ToolError::invalid_input("This PDF has no pages. Nothing was changed.")); }
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
        if !matches!(request.output_location,OutputLocation::AlongsideInput) { return Err("Scene exports must be alongside the input".into()); }
        let mut bytes=Vec::new();
        compose(&input,&request.scene)?.save_to(&mut bytes).map_err(err)?;
        let parent=input.parent().ok_or("Input has no parent directory")?;
        let stem=input.file_stem().ok_or("Input has no filename")?.to_string_lossy();
        for n in 1..=10000 {
            let candidate=parent.join(format!("{stem}-edited-{n}.pdf"));
            let reservation=match OutputNaming::reserve_named_candidate(&candidate).map_err(err)? {
                Some(reservation)=>reservation,
                None=>continue,
            };
            let mut file=OpenOptions::new().write(true).open(reservation.path()).map_err(err)?;
            file.write_all(&bytes).and_then(|_|file.sync_all()).map_err(err)?;
            return reservation.publish().map_err(err);
        } Err("Could not reserve a unique export filename".into())
    })();
    match result { Ok(path)=>JobOutcome::succeeded(input, vec![path], "PDF scene exported"),Err(e)=>JobOutcome::failure(input,ToolError::processing(e)) }
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
        SceneObject { kind, rect: Rect{x:30.,y:40.,width:120.,height:40.}, text:"LOCAL".into(),font_size:18.,color:"#0066CC".into(),opacity:0.5,
            strokes:vec![vec![Point{x:0.,y:0.5},Point{x:0.5,y:0.},Point{x:1.,y:1.}]], shape:None, highlight_mode:None,
            signature_mode:None, signature_path:None, font_family:None, watermark_pattern:None }
    }
    fn scene() -> PdfScene {
        PdfScene { pages:vec![
            ScenePage {source_index:Some(1),width:612.,height:792.,rotation:180, mirrored: false,crop:Some(Rect{x:10.,y:20.,width:500.,height:600.}),source_rotation:None,source_box:None,objects:vec![object(Kind::Highlight),object(Kind::Text),object(Kind::Shape),object(Kind::Watermark),object(Kind::Signature)]},
            ScenePage {source_index:None,width:300.,height:200.,rotation:90, mirrored: false,crop:None,source_rotation:None,source_box:None,objects:vec![object(Kind::Text)]},
            ScenePage {source_index:Some(0),width:300.,height:200.,rotation:0, mirrored: false,crop:None,source_rotation:None,source_box:None,objects:vec![]},
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
        assert!(content.contains("/S4 gs")); assert!(content.contains("re W n")); assert!(content.contains("<4C4F43414C>"));
        let blank=composed.get_page_content(pages[1]); assert!(!String::from_utf8_lossy(&blank).contains("/Original Do"));
        assert!(String::from_utf8_lossy(&blank).contains("<4C4F43414C>"));
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
    fn diagonal_watermark_matrices_match_react_y_down_angles() {
        let area = Rect { x: 0., y: 0., width: 612., height: 792. };
        let anchor = Rect { x: 100., y: 200., width: 300., height: 80. };
        for (pattern, expected_angle) in [
            (WatermarkPattern::BottomRightToTopLeft, -45_f32),
            (WatermarkPattern::TopRightToBottomLeft, 45_f32),
        ] {
            let placement = watermark_placements(&area, &anchor, 18., "LOCAL", Some(&pattern))[0];
            assert_eq!(placement.2, expected_angle);
            let [a, b, c, d] = scene_angle_to_pdf_text_matrix(placement.2);
            let radians = expected_angle.to_radians();
            assert!((a - radians.cos()).abs() < 0.0001);
            assert!((-b - radians.sin()).abs() < 0.0001);
            assert!((-c - radians.sin()).abs() < 0.0001);
            assert!((-d - radians.cos()).abs() < 0.0001);
        }
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
            source_index: Some(page.index), width: page.width, height: page.height, rotation: 0, mirrored: false, crop: None,
            source_rotation: Some(page.rotation),
            source_box: Some(Rect { x: page.page_box[0], y: page.page_box[1], width: page.page_box[2] - page.page_box[0], height: page.page_box[3] - page.page_box[1] }),
            objects: vec![],
        }).collect() };
        let outcome = export(&ExportRequest { paths: vec![input.clone()], scene, output_location: OutputLocation::AlongsideInput }, input.clone());
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
            source_index: Some(0), width: 300., height: 200., rotation: 0, mirrored: false, crop: None,
            source_rotation: Some(90), source_box: Some(Rect { x: 20., y: 30., width: 200., height: 300. }),
            objects: vec![object(Kind::Text)],
        }] };
        let outcome = export(&ExportRequest { paths: vec![input.clone()], scene, output_location: OutputLocation::AlongsideInput }, input.clone());
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
        assert!(content.contains("<4C4F43414C>"));
        assert!(content.contains("0 1 1 0 20 30 cm"));
        assert_eq!(fs::read(&input).unwrap(), original);
    }
    #[test]
    fn exports_preserve_original_and_existing_outputs_byte_for_byte() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); fixture(&input);
        let original=fs::read(&input).unwrap();
        let occupied=temp.0.join("source-edited-1.pdf"); fs::write(&occupied,b"existing output").unwrap();
        let request=ExportRequest {paths:vec![input.clone()],scene:scene(),output_location:OutputLocation::AlongsideInput};
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
        let result=export(&ExportRequest {paths:vec![],scene:s,output_location:OutputLocation::AlongsideInput},input.clone());
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
        let request=ExportRequest{paths:vec![input.clone()],scene:scene(),output_location:OutputLocation::AlongsideInput};
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
    fn renders_shape_variants_and_fixed_watermark_patterns() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); fixture(&input);
        let variants=[Shape::Square,Shape::Round,Shape::Triangle,Shape::Line,Shape::DottedLine,Shape::ArrowLeft,Shape::ArrowRight,Shape::ArrowUp,Shape::ArrowDown];
        let mut objects=variants.into_iter().map(|shape| { let mut value=object(Kind::Shape); value.shape=Some(shape); value }).collect::<Vec<_>>();
        let mut watermark=object(Kind::Watermark); watermark.watermark_pattern=Some(WatermarkPattern::CenterVertical); objects.push(watermark);
        let mut model=scene(); model.pages[0].objects=objects;
        let composed=compose(&input,&model).unwrap(); let page_id=composed.get_pages().values().next().copied().unwrap();
        let content_bytes=composed.get_page_content(page_id); let content=String::from_utf8_lossy(&content_bytes);
        assert!(content.contains("[3 4] 0 d"));
        assert!(content.contains("BT /SceneFont 18 Tf"));
        assert!(content.contains("m ")); assert!(content.contains(" h f"));
    }

    #[test]
    fn watermark_placements_follow_moved_and_resized_rectangles() {
        let area = Rect { x: 10., y: 20., width: 500., height: 600. };
        let original = Rect { x: 30., y: 40., width: 120., height: 40. };
        let moved = Rect { x: 180., y: 220., width: 120., height: 40. };
        let resized = Rect { x: 30., y: 40., width: 240., height: 80. };
        let patterns = [
            WatermarkPattern::AcrossPage,
            WatermarkPattern::BottomRightToTopLeft,
            WatermarkPattern::TopRightToBottomLeft,
            WatermarkPattern::CenterHorizontal,
            WatermarkPattern::CenterVertical,
        ];
        for pattern in patterns {
            let initial = watermark_placements(&area, &original, 18., "LOCAL", Some(&pattern));
            let moved_placements = watermark_placements(&area, &moved, 18., "LOCAL", Some(&pattern));
            let resized_placements = watermark_placements(&area, &resized, 36., "LOCAL", Some(&pattern));
            assert_ne!(initial, moved_placements, "moving {pattern:?} did not change native placements");
            assert_ne!(initial, resized_placements, "resizing {pattern:?} did not change native placements");
        }
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

    #[test]
    fn cursive_typed_signatures_use_bundled_raster_fonts() {
        let temp = TempDir::new().unwrap();
        let input = temp.0.join("source.pdf");
        fixture(&input);
        for font_family in ["Satisfy", "Pacifico"] {
            let mut value = object(Kind::Signature);
            value.signature_mode = Some(SignatureMode::Text);
            value.text = "A. Local".into();
            value.font_family = Some(font_family.into());
            let mut model = scene();
            model.pages[0].objects = vec![value];
            let composed = compose(&input, &model).unwrap();
            let page_id = composed.get_pages().values().next().copied().unwrap();
            let content = String::from_utf8_lossy(&composed.get_page_content(page_id)).into_owned();
            assert!(content.contains("/Sig0 Do"), "{font_family} signature was not rasterized: {content}");
            let resources = composed.get_dictionary(page_id).unwrap().get(b"Resources").unwrap().as_dict().unwrap();
            let xobjects = resources.get(b"XObject").unwrap().as_dict().unwrap();
            let signature = xobjects.get(b"Sig0").unwrap().as_reference().unwrap();
            let signature = composed.get_object(signature).unwrap().as_stream().unwrap();
            assert_eq!(signature.dict.get(b"Subtype").unwrap().as_name().unwrap(), b"Image");
            assert!(signature.dict.has(b"SMask"), "{font_family} signature lost transparency");
        }
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
            source_index: Some(0), width: 612., height: 792., rotation: 0, mirrored: false,
            crop: Some(Rect { x: 0., y: 0., width: 500., height: 700. }),
            source_rotation: None, source_box: None,
            objects: vec![object(Kind::Text), signature_object],
        }] };

        let composed = compose(&input, &model).unwrap();
        let page = composed.get_pages().values().next().copied().unwrap();
        let content = String::from_utf8_lossy(&composed.get_page_content(page)).into_owned();
        assert!(content.contains("/SceneFont2 18 Tf"), "font resource collision: {content}");
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
        let scene=PdfScene { pages: pages.iter().enumerate().map(|(index,_)| ScenePage { source_index:Some(index), width:if index==0 {200.} else {612.}, height:if index==0 {300.} else {792.}, rotation:0, mirrored: false, crop:None, source_rotation:None, source_box:None, objects:vec![] }).collect() };
        let composed=compose(&input,&scene).unwrap();
        let catalog=composed.get_dictionary(catalog_id).unwrap();
        for key in [b"Metadata".as_slice(),b"Names",b"Outlines",b"StructTreeRoot",b"AcroForm"] { assert!(catalog.has(key),"catalog lost {key:?}"); }
        let output_pages:Vec<_>=composed.get_pages().values().copied().collect();
        assert_eq!(output_pages, pages);
        let annots=composed.get_dictionary(output_pages[1]).unwrap().get(b"Annots").unwrap().as_array().unwrap();
        assert_eq!(annots.len(),2);
        assert_eq!(fs::read(&input).unwrap(),original);
    }

    /// Six letter-size pages. Each has its own content, a note, a link to the next page and a form field.
    fn six_page_linked_form_fixture(path: &Path) {
        let mut d = Document::with_version("1.7");
        let root = d.new_object_id();
        let font = d.add_object(dictionary! {"Type"=>"Font", "Subtype"=>"Type1", "BaseFont"=>"Helvetica"});
        let fonts = d.add_object(dictionary! {"F1"=>font});
        let resources = d.add_object(dictionary! {"Font"=>fonts});
        let pages: Vec<ObjectId> = (0..6).map(|_| d.new_object_id()).collect();
        let mut fields = Vec::new();
        for (index, page) in pages.iter().enumerate() {
            let content = d.add_object(Stream::new(dictionary!{}, format!("BT /F1 18 Tf 40 700 Td (Source page {}) Tj ET", index + 1).into_bytes()));
            let note = d.add_object(dictionary! {"Type"=>"Annot", "Subtype"=>"Text", "Rect"=>array(&[30., 40., 90., 80.]), "Contents"=>Object::string_literal(format!("Note {}", index + 1))});
            let link = d.add_object(dictionary! {"Type"=>"Annot", "Subtype"=>"Link", "Rect"=>array(&[100., 40., 160., 80.]),
                "Dest"=>vec![Object::Reference(pages[(index + 1) % 6]), Object::Name(b"Fit".to_vec())]});
            let widget = d.add_object(dictionary! {"Type"=>"Annot", "Subtype"=>"Widget", "FT"=>"Tx", "T"=>Object::string_literal(format!("field{}", index + 1)),
                "Rect"=>array(&[170., 40., 260., 80.]), "P"=>Object::Reference(*page)});
            fields.push(Object::Reference(widget));
            d.objects.insert(*page, Object::Dictionary(dictionary! {"Type"=>"Page", "Parent"=>root, "MediaBox"=>array(&[0., 0., 612., 792.]), "Resources"=>resources,
                "Contents"=>content, "Annots"=>vec![Object::Reference(note), Object::Reference(link), Object::Reference(widget)]}));
        }
        d.objects.insert(root, Object::Dictionary(dictionary! {"Type"=>"Pages", "Kids"=>pages.iter().map(|page| Object::Reference(*page)).collect::<Vec<_>>(), "Count"=>6}));
        let acro_form = d.add_object(dictionary! {"Fields"=>fields});
        let catalog = d.add_object(dictionary! {"Type"=>"Catalog", "Pages"=>root, "AcroForm"=>acro_form});
        d.trailer.set("Root", catalog);
        d.save(path).unwrap();
    }

    #[test]
    fn block_moved_reversed_and_interleaved_orders_only_reorder_the_page_tree() {
        let temp = TempDir::new().unwrap();
        let input = temp.0.join("six.pdf");
        six_page_linked_form_fixture(&input);
        let original_bytes = fs::read(&input).unwrap();
        let original = Document::load(&input).unwrap();
        let pages: Vec<_> = original.get_pages().values().copied().collect();
        let catalog = original.trailer.get(b"Root").unwrap().as_reference().unwrap();
        let pages_root = original.get_dictionary(catalog).unwrap().get(b"Pages").unwrap().as_reference().unwrap();
        let scene = |order: &[usize]| PdfScene { pages: order.iter().map(|&index| ScenePage {
            source_index: Some(index), width: 612., height: 792., rotation: 0, mirrored: false, crop: None,
            source_rotation: Some(0), source_box: Some(Rect { x: 0., y: 0., width: 612., height: 792. }), objects: vec![],
        }).collect() };
        // Pages 2 and 3 moved as a block to the end; the whole document reversed; a two-sided scan interleaved.
        for order in [[0, 3, 4, 5, 1, 2], [5, 4, 3, 2, 1, 0], [0, 5, 1, 4, 2, 3]] {
            let expected: Vec<_> = order.iter().map(|&index| pages[index]).collect();
            let composed = compose(&input, &scene(&order)).expect("a reorder should compose");
            let reordered: Vec<_> = composed.get_pages().values().copied().collect();
            assert_eq!(reordered, expected);
            // Content, notes, links and form fields are the same objects, byte for byte; only the page tree's order changes.
            assert_only_page_tree_order_changed(&composed, &original, pages_root, &reordered);

            let request = ExportRequest { paths: vec![input.clone()], scene: scene(&order), output_location: OutputLocation::AlongsideInput };
            let first = export(&request, input.clone());
            let second = export(&request, input.clone());
            assert!(first.error().is_none(), "{:?}", first.error());
            assert!(second.error().is_none(), "{:?}", second.error());
            assert_ne!(first.output_paths(), second.output_paths());
            let bytes = fs::read(&first.output_paths()[0]).unwrap();
            assert_eq!(bytes, fs::read(&second.output_paths()[0]).unwrap(), "two exports of the same order differ");
            let output = Document::load(&first.output_paths()[0]).unwrap();
            let output_pages: Vec<_> = output.get_pages().values().copied().collect();
            for (position, &source) in order.iter().enumerate() {
                let content = output.get_page_content(output_pages[position]);
                assert!(String::from_utf8_lossy(&content).contains(&format!("(Source page {})", source + 1)));
                let annotations = output.get_dictionary(output_pages[position]).unwrap().get(b"Annots").unwrap().as_array().unwrap();
                assert_eq!(annotations.len(), 3);
                let link = output.get_dictionary(annotations[1].as_reference().unwrap()).unwrap();
                let target = link.get(b"Dest").unwrap().as_array().unwrap()[0].as_reference().unwrap();
                assert_eq!(target, pages[(source + 1) % 6], "a link lost its target page");
            }
            let output_catalog = output.trailer.get(b"Root").unwrap().as_reference().unwrap();
            let acro_form = output.get_dictionary(output_catalog).unwrap().get(b"AcroForm").unwrap().as_reference().unwrap();
            assert_eq!(output.get_dictionary(acro_form).unwrap().get(b"Fields").unwrap().as_array().unwrap().len(), 6);
            assert_eq!(fs::read(&input).unwrap(), original_bytes);
        }
    }

    #[test]
    fn each_quarter_turn_adds_to_the_source_rotation_and_keeps_annotations_fields_and_destinations() {
        let temp = TempDir::new().unwrap();
        let input = temp.0.join("annotated-rotated-offset.pdf");
        annotated_rotated_offset_fixture(&input);
        let original_bytes = fs::read(&input).unwrap();
        let original = Document::load(&input).unwrap();
        let page = original.get_pages().values().next().copied().unwrap();
        let original_annots = original.get_dictionary(page).unwrap().get(b"Annots").unwrap().clone();
        // The source page is /Rotate 90, inherited from its page tree. Each scene quarter turn is added to it.
        for (rotation, expected) in [(90, 180), (180, 270), (270, 0), (-90, 0)] {
            let scene = PdfScene { pages: vec![ScenePage {
                source_index: Some(0), width: 300., height: 200., rotation, crop: None,
                source_rotation: Some(90), mirrored: false, source_box: Some(Rect { x: 20., y: 30., width: 200., height: 300. }), objects: vec![],
            }] };
            let outcome = export(&ExportRequest { paths: vec![input.clone()], scene, output_location: OutputLocation::AlongsideInput }, input.clone());
            assert!(outcome.error().is_none(), "rotation {rotation}: {:?}", outcome.error());
            let output = Document::load(&outcome.output_paths()[0]).unwrap();
            let output_page = output.get_pages().values().next().copied().unwrap();
            assert_eq!(output_page, page, "rotation {rotation} replaced the source page");
            assert_eq!(inherited(&output, output_page, b"Rotate").unwrap().unwrap().as_i64().unwrap(), expected, "rotation {rotation}");
            // Lossless: same content, same boxes, and the notes, link and form field stay on the page with their own coordinates.
            assert_eq!(output.get_page_content(output_page), original.get_page_content(page));
            assert_eq!(bounds(&output, &inherited(&output, output_page, b"CropBox").unwrap().unwrap()).unwrap(), [20., 30., 220., 330.]);
            let page_dictionary = output.get_dictionary(output_page).unwrap();
            assert_eq!(page_dictionary.get(b"Annots").unwrap(), &original_annots);
            for annotation in original_annots.as_array().unwrap() {
                let id = annotation.as_reference().unwrap();
                assert_eq!(output.get_object(id).unwrap(), original.get_object(id).unwrap());
            }
            let catalog = output.get_dictionary(output.trailer.get(b"Root").unwrap().as_reference().unwrap()).unwrap();
            for key in [b"AcroForm".as_slice(), b"Names", b"Outlines"] { assert!(catalog.has(key), "rotation {rotation} lost {key:?}"); }
            assert_eq!(fs::read(&input).unwrap(), original_bytes, "rotation {rotation} changed the original");
        }
    }

    #[test]
    fn rotating_reordered_pages_only_sets_their_rotate_entry() {
        let temp = TempDir::new().unwrap();
        let input = temp.0.join("six.pdf");
        six_page_linked_form_fixture(&input);
        let original_bytes = fs::read(&input).unwrap();
        let original = Document::load(&input).unwrap();
        let pages: Vec<_> = original.get_pages().values().copied().collect();
        let order = [5, 4, 3, 2, 1, 0];
        let rotations = [0, 90, 180, 270, 0, 0];
        let scene = PdfScene { pages: order.iter().zip(rotations).map(|(&index, rotation)| ScenePage {
            source_index: Some(index), width: 612., height: 792., rotation, crop: None,
            source_rotation: Some(0), mirrored: false, source_box: Some(Rect { x: 0., y: 0., width: 612., height: 792. }), objects: vec![],
        }).collect() };
        let composed = compose(&input, &scene).expect("a rotated reorder should compose");
        let output_pages: Vec<_> = composed.get_pages().values().copied().collect();
        assert_eq!(output_pages, order.iter().map(|&index| pages[index]).collect::<Vec<_>>());
        assert_eq!(composed.objects.len(), original.objects.len(), "rotation added objects");
        for (position, &index) in order.iter().enumerate() {
            let mut expected = original.get_dictionary(pages[index]).unwrap().clone();
            if rotations[position] != 0 { expected.set("Rotate", rotations[position] as i64); }
            assert_eq!(composed.get_dictionary(pages[index]).unwrap(), &expected, "page {} changed beyond its rotation", index + 1);
        }
        let outcome = export(&ExportRequest { paths: vec![input.clone()], scene, output_location: OutputLocation::AlongsideInput }, input.clone());
        assert!(outcome.error().is_none(), "{:?}", outcome.error());
        assert_eq!(fs::read(&input).unwrap(), original_bytes);
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
                source_index: Some(1), width: 612., height: 792., rotation: 0, mirrored: false, crop: None,
                source_rotation: Some(0), source_box: Some(Rect { x: 0., y: 0., width: 612., height: 792. }), objects: vec![],
            },
            ScenePage {
                source_index: Some(0), width: 300., height: 200., rotation: 0, mirrored: false, crop: None,
                source_rotation: Some(90), source_box: Some(Rect { x: 20., y: 30., width: 200., height: 300. }), objects: vec![],
            },
        ] };

        let composed = compose(&input, &scene).expect("tagged pure reorder should compose");
        let reordered: Vec<_> = composed.get_pages().values().copied().collect();
        assert_eq!(reordered, vec![pages[1], pages[0]]);
        assert_only_page_tree_order_changed(&composed, &original, pages_root, &reordered);
        assert_eq!(fs::read(&input).unwrap(), original_bytes);
    }

    /// A portrait page with a red mark near its top-left corner and three words, once upright and
    /// once as /Rotate 90 with an offset CropBox, so the flip is checked in both page spaces.
    fn mirror_mark_fixture(path: &Path) {
        let mut d = Document::with_version("1.7");
        let root = d.new_object_id();
        let font = d.add_object(dictionary! {"Type"=>"Font", "Subtype"=>"Type1", "BaseFont"=>"Helvetica"});
        let resources = d.add_object(dictionary! {"Font"=>dictionary! {"F1"=>font}});
        let upright = d.add_object(Stream::new(dictionary!{}, b"q 1 0 0 rg 10 240 50 50 re f Q BT /F1 16 Tf 30 140 Td (Alpha Beta Gamma) Tj ET".to_vec()));
        let turned = d.add_object(Stream::new(dictionary!{}, b"q 1 0 0 rg 30 270 50 50 re f Q BT /F1 16 Tf 50 170 Td (Alpha Beta Gamma) Tj ET".to_vec()));
        let first = d.add_object(dictionary! {"Type"=>"Page", "Parent"=>root, "MediaBox"=>array(&[0.,0.,200.,300.]), "Resources"=>resources, "Contents"=>upright});
        let second = d.add_object(dictionary! {"Type"=>"Page", "Parent"=>root, "MediaBox"=>array(&[10.,20.,250.,360.]), "CropBox"=>array(&[20.,30.,220.,330.]), "Rotate"=>90, "Resources"=>resources, "Contents"=>turned});
        d.objects.insert(root, Object::Dictionary(dictionary! {"Type"=>"Pages", "Kids"=>vec![Object::Reference(first),Object::Reference(second)], "Count"=>2}));
        let catalog = d.add_object(dictionary! {"Type"=>"Catalog","Pages"=>root});
        d.trailer.set("Root",catalog);
        d.save(path).unwrap();
    }
    fn oriented_page(source_index: usize, (width, height): (f32, f32), rotation: i32, mirrored: bool) -> ScenePage {
        ScenePage { source_index: Some(source_index), width, height, rotation, mirrored, crop: None, source_rotation: None, source_box: None, objects: vec![] }
    }
    /// The centre of the red mark, as fractions of the rendered page's width and height.
    fn red_mark_centre(preview: &ScenePreview) -> (f32, f32) {
        let bytes = base64::engine::general_purpose::STANDARD.decode(preview.data_url.trim_start_matches("data:image/png;base64,")).unwrap();
        let pixels = image::load_from_memory(&bytes).unwrap().to_rgba8();
        let (mut sum_x, mut sum_y, mut count) = (0f32, 0f32, 0f32);
        for (x, y, pixel) in pixels.enumerate_pixels() {
            if pixel[0] > 200 && pixel[1] < 80 && pixel[2] < 80 { sum_x += x as f32; sum_y += y as f32; count += 1.; }
        }
        assert!(count > 0., "the red mark is missing");
        (sum_x / count / pixels.width() as f32, sum_y / count / pixels.height() as f32)
    }
    /// Where a point of the upright page lands once it is flipped left to right (when mirrored) and then turned clockwise.
    fn displayed_point((mut x, mut y): (f32, f32), rotation: i32, mirrored: bool) -> (f32, f32) {
        if mirrored { x = 1. - x; }
        for _ in 0..rotation.rem_euclid(360) / 90 { (x, y) = (1. - y, x); }
        (x, y)
    }
    fn image_xobject_count(document: &Document) -> usize {
        document.objects.values().filter(|object| matches!(object, Object::Stream(stream) if stream.dict.get(b"Subtype").ok().and_then(|value| value.as_name().ok()) == Some(b"Image"))).count()
    }

    #[test]
    fn mirroring_flips_the_page_as_a_vector_in_all_eight_orientations() {
        let _guard = crate::kit::lock_process_env();
        let renderer = match renderer() { Ok(r) => r, Err(e) => { if std::env::var_os("TOOLBOX_REQUIRE_PDF_RENDERER").is_some() { panic!("{e}") }; eprintln!("Renderer unavailable: {e}"); return; } };
        let temp = TempDir::new().unwrap();
        let input = temp.0.join("mark.pdf");
        mirror_mark_fixture(&input);
        let original_bytes = fs::read(&input).unwrap();
        let original = Document::load(&input).unwrap();
        // Where the mark shows on the unedited page, from the fixture's geometry: 10–60 × 240–290 on the
        // upright 200 × 300 page, and 30–80 × 270–320 inside the /Rotate 90 page's CropBox 20–220 × 30–330.
        // The turned page's unmirrored orientations keep its CropBox, which the test renderer ignores,
        // so only its mirrored ones (redrawn on a page the size of the visible area) are measured.
        for (source_index, size, upright, flips) in [(0, (200., 300.), (35. / 200., 35. / 300.), &[false, true][..]), (1, (300., 200.), (265. / 300., 35. / 200.), &[true][..])] {
            for rotation in [0, 90, 180, 270] {
                for &mirrored in flips {
                    let scene = PdfScene { pages: vec![oriented_page(source_index, size, rotation, mirrored)] };
                    let mut composed = compose(&input, &scene).unwrap();
                    assert_eq!(image_xobject_count(&composed), image_xobject_count(&original), "page {source_index} {rotation}° mirrored {mirrored}: a raster image was added");
                    let path = temp.0.join(format!("oriented-{source_index}-{rotation}-{mirrored}.pdf"));
                    composed.save(&path).unwrap();
                    let actual = red_mark_centre(&render(&path, 0, &renderer, &temp).unwrap());
                    let expected = displayed_point(upright, rotation, mirrored);
                    assert!((actual.0 - expected.0).abs() < 0.03 && (actual.1 - expected.1).abs() < 0.03,
                        "page {source_index} {rotation}° mirrored {mirrored}: mark at {actual:?}, expected {expected:?}");
                    // The upright page's mark starts top-left. Left to right moves it top-right; top to bottom
                    // (stored as left to right turned 180°) moves it bottom-left.
                    if source_index == 0 && mirrored && rotation == 0 { assert!(actual.0 > 0.5 && actual.1 < 0.5, "left to right: {actual:?}"); }
                    if source_index == 0 && mirrored && rotation == 180 { assert!(actual.0 < 0.5 && actual.1 > 0.5, "top to bottom: {actual:?}"); }
                }
            }
        }
        assert_eq!(fs::read(&input).unwrap(), original_bytes, "mirroring changed the original");
    }

    #[test]
    fn mirrored_export_keeps_text_extractable_in_reading_order_and_is_deterministic() {
        let temp = TempDir::new().unwrap();
        let input = temp.0.join("mark.pdf");
        mirror_mark_fixture(&input);
        let original_bytes = fs::read(&input).unwrap();
        let scene = || PdfScene { pages: vec![oriented_page(0, (200., 300.), 0, true), oriented_page(1, (300., 200.), 180, true)] };
        let first = export(&ExportRequest { paths: vec![input.clone()], scene: scene(), output_location: OutputLocation::AlongsideInput }, input.clone());
        let second = export(&ExportRequest { paths: vec![input.clone()], scene: scene(), output_location: OutputLocation::AlongsideInput }, input.clone());
        assert!(first.error().is_none() && second.error().is_none(), "{:?} {:?}", first.error(), second.error());
        assert_ne!(first.output_paths()[0], second.output_paths()[0], "a second export must not overwrite the first");
        assert_eq!(fs::read(&first.output_paths()[0]).unwrap(), fs::read(&second.output_paths()[0]).unwrap(), "mirrored export is not byte-identical across runs");
        assert_eq!(fs::read(&input).unwrap(), original_bytes, "mirroring changed the original");
        let output = Document::load(&first.output_paths()[0]).unwrap();
        assert_eq!(image_xobject_count(&output), 0, "a raster image was added");
        // The words stay text, in their content order. A position-sorted extraction reads a mirrored line
        // backwards, as it reads any flipped glyphs, so this reads the content order the way copy and search do.
        let Some(extractor) = text_extractor() else { return; };
        for page in ["1", "2"] {
            let output = Command::new(&extractor).args(["-raw", "-enc", "UTF-8", "-f", page, "-l", page]).arg(&first.output_paths()[0]).arg("-").output().unwrap();
            assert!(output.status.success());
            let text: String = String::from_utf8(output.stdout).unwrap().split_whitespace().collect();
            assert!(text.contains("AlphaBetaGamma"), "page {page} text: {text:?}");
        }
    }

    /// Page 1 carries a link with quad points and an /XYZ destination, a text field with an appearance
    /// stream, and is the target of a bookmark, a named destination tree, a catalog /Dests entry, the
    /// open action and a GoTo link on page 2. `rotated` gives page 1 an offset /Rotate 90 box.
    fn mirror_navigation_fixture(path: &Path, rotated: bool) -> (ObjectId, ObjectId, ObjectId) {
        let mut d = Document::with_version("1.7");
        let root = d.new_object_id();
        let first = d.new_object_id();
        let content = d.add_object(Stream::new(dictionary!{}, b"q 1 0 0 rg 30 40 60 40 re f Q".to_vec()));
        let link = d.add_object(dictionary! {"Type"=>"Annot", "Subtype"=>"Link", "Rect"=>array(&[30., 40., 90., 80.]),
            "QuadPoints"=>array(&[30., 80., 90., 80., 30., 40., 90., 40.]),
            "Dest"=>vec![Object::Reference(first), Object::Name(b"XYZ".to_vec()), 50.into(), 250.into(), 0.into()]});
        let appearance = d.add_object(Stream::new(dictionary! {"Type"=>"XObject", "Subtype"=>"Form", "BBox"=>array(&[0., 0., 60., 40.])}, b"0 0 1 rg 0 0 60 40 re f".to_vec()));
        let widget = d.add_object(dictionary! {"Type"=>"Annot", "Subtype"=>"Widget", "FT"=>"Tx", "T"=>Object::string_literal("name"),
            "Rect"=>array(&[100., 40., 160., 80.]), "P"=>Object::Reference(first), "AP"=>dictionary! {"N"=>Object::Reference(appearance)}, "MK"=>dictionary! {"R"=>90}});
        let mut page = dictionary! {"Type"=>"Page", "Parent"=>root, "Contents"=>content, "Annots"=>vec![Object::Reference(link), Object::Reference(widget)]};
        if rotated {
            page.set("MediaBox", array(&[10., 20., 250., 360.])); page.set("CropBox", array(&[20., 30., 220., 330.])); page.set("Rotate", 90);
        } else {
            page.set("MediaBox", array(&[0., 0., 200., 300.]));
        }
        d.objects.insert(first, Object::Dictionary(page));
        let goto = d.add_object(dictionary! {"Type"=>"Annot", "Subtype"=>"Link", "Rect"=>array(&[10., 10., 60., 30.]),
            "A"=>dictionary! {"S"=>"GoTo", "D"=>vec![Object::Reference(first), Object::Name(b"FitR".to_vec()), 30.into(), 40.into(), 90.into(), 80.into()]}});
        let second = d.add_object(dictionary! {"Type"=>"Page", "Parent"=>root, "MediaBox"=>array(&[0., 0., 200., 300.]), "Annots"=>vec![Object::Reference(goto)]});
        d.objects.insert(root, Object::Dictionary(dictionary! {"Type"=>"Pages", "Kids"=>vec![Object::Reference(first), Object::Reference(second)], "Count"=>2}));
        let outlines = d.new_object_id();
        let bookmark = d.add_object(dictionary! {"Title"=>Object::string_literal("Section"), "Parent"=>outlines,
            "Dest"=>vec![Object::Reference(first), Object::Name(b"XYZ".to_vec()), 40.into(), 200.into(), Object::Null]});
        d.objects.insert(outlines, Object::Dictionary(dictionary! {"Type"=>"Outlines", "First"=>bookmark, "Last"=>bookmark, "Count"=>1}));
        let names = d.add_object(dictionary! {"Dests"=>dictionary! {"Names"=>vec![Object::string_literal("top"),
            Object::Array(vec![Object::Reference(first), Object::Name(b"FitH".to_vec()), 250.into()])]}});
        let dests = d.add_object(dictionary! {"side"=>vec![Object::Reference(first), Object::Name(b"FitV".to_vec()), 60.into()]});
        let acro_form = d.add_object(dictionary! {"Fields"=>vec![Object::Reference(widget)]});
        let catalog = d.add_object(dictionary! {"Type"=>"Catalog", "Pages"=>root, "Outlines"=>outlines, "Names"=>names, "Dests"=>dests,
            "OpenAction"=>vec![Object::Reference(first), Object::Name(b"FitBH".to_vec()), 250.into()], "AcroForm"=>acro_form});
        d.trailer.set("Root", catalog);
        d.save(path).unwrap();
        (first, widget, appearance)
    }
    fn numbers_of(document: &Document, value: &Object) -> Vec<Option<f32>> {
        resolve(document, value).unwrap().as_array().unwrap().iter().map(|value| match value { Object::Null => None, value => Some(super::super::metadata::number(value).unwrap()) }).collect()
    }
    fn assert_close(actual: &[Option<f32>], expected: &[Option<f32>], what: &str) {
        assert_eq!(actual.len(), expected.len(), "{what}: {actual:?} vs {expected:?}");
        for (a, e) in actual.iter().zip(expected) {
            match (a, e) { (Some(a), Some(e)) => assert!((a - e).abs() < 0.01, "{what}: {actual:?} vs {expected:?}"), (a, e) => assert_eq!(a, e, "{what}") }
        }
    }
    /// A destination array's view name and its coordinates.
    fn destination_of(document: &Document, value: &Object) -> (Vec<u8>, Vec<Option<f32>>) {
        let items = resolve(document, value).unwrap().as_array().unwrap();
        let coordinates = Object::Array(items[2..].to_vec());
        (items[1].as_name().unwrap().to_vec(), numbers_of(document, &coordinates))
    }

    #[test]
    fn mirroring_moves_annotations_widgets_and_destinations_with_the_content() {
        let _guard = crate::kit::lock_process_env();
        let renderer = match renderer() { Ok(r) => Some(r), Err(e) => { if std::env::var_os("TOOLBOX_REQUIRE_PDF_RENDERER").is_some() { panic!("{e}") }; eprintln!("Renderer unavailable: {e}"); None } };
        // Where a source point lands on the mirrored page, worked out by hand from the page boxes. The
        // upright page flips across its 200-wide box. The /Rotate 90 page's visible area is its CropBox
        // 20–220 × 30–330 turned clockwise (300 × 200), so x reads down from its top and y reads across;
        // the mirror then flips that across the 300-wide visible area.
        let upright = |(x, y): (f32, f32)| (200. - x, y);
        let turned = |(x, y): (f32, f32)| (300. - (y - 30.), 200. - (x - 20.));
        for rotated in [false, true] {
            let place: &dyn Fn((f32, f32)) -> (f32, f32) = if rotated { &turned } else { &upright };
            let rect = |[x1, y1, x2, y2]: [f32; 4]| {
                let ((ax, ay), (bx, by)) = (place((x1, y1)), place((x2, y2)));
                [Some(ax.min(bx)), Some(ay.min(by)), Some(ax.max(bx)), Some(ay.max(by))]
            };
            let size = if rotated { (300., 200.) } else { (200., 300.) };
            // Left to right, top to bottom (stored as left to right turned 180°), and both with a quarter turn.
            for rotation in [0, 180, 90, 270] {
                let what = format!("rotated source {rotated}, turn {rotation}");
                let temp = TempDir::new().unwrap();
                let input = temp.0.join("annotated.pdf");
                let (page, widget, appearance) = mirror_navigation_fixture(&input, rotated);
                let original = fs::read(&input).unwrap();
                let scene = PdfScene { pages: vec![oriented_page(0, size, rotation, true), oriented_page(1, (200., 300.), 0, false)] };
                let result = export(&ExportRequest { paths: vec![input.clone()], scene, output_location: OutputLocation::AlongsideInput }, input.clone());
                assert!(result.error().is_none(), "{what}: {:?}", result.error());
                assert_eq!(fs::read(&input).unwrap(), original, "{what}: mirroring changed the original");
                let output = Document::load(&result.output_paths()[0]).unwrap();
                let pages: Vec<_> = output.get_pages().values().copied().collect();
                assert_eq!(pages[0], page, "{what}: the mirrored page lost its object id, so destinations would dangle");
                let page_dict = output.get_dictionary(page).unwrap();
                assert_eq!(page_dict.get(b"Rotate").unwrap().as_i64().unwrap(), rotation as i64, "{what}");
                assert_close(&numbers_of(&output, page_dict.get(b"MediaBox").unwrap()), &[Some(0.), Some(0.), Some(size.0), Some(size.1)], &what);

                // Annotations sit over the mirrored content, and the field stays a live form field.
                let annots = page_dict.get(b"Annots").unwrap().as_array().unwrap();
                assert_eq!(annots.len(), 2, "{what}");
                let link = output.get_dictionary(annots[0].as_reference().unwrap()).unwrap();
                let link_rect = numbers_of(&output, link.get(b"Rect").unwrap());
                assert_close(&link_rect, &rect([30., 40., 90., 80.]), &what);
                // The fixture's red mark fills the link's source rectangle, so on the rendered page the
                // mirrored link must still cover it.
                if let Some(renderer) = &renderer {
                    let mark = red_mark_centre(&render(&result.output_paths()[0], 0, renderer, &temp).unwrap());
                    let centre = (link_rect[0].unwrap() + link_rect[2].unwrap()) / 2. / size.0;
                    let middle = 1. - (link_rect[1].unwrap() + link_rect[3].unwrap()) / 2. / size.1;
                    let expected = displayed_point((centre, middle), rotation, false);
                    assert!((mark.0 - expected.0).abs() < 0.03 && (mark.1 - expected.1).abs() < 0.03, "{what}: mark at {mark:?}, link at {expected:?}");
                }
                let quads = [(30., 80.), (90., 80.), (30., 40.), (90., 40.)].into_iter().flat_map(|point| { let (x, y) = place(point); [Some(x), Some(y)] }).collect::<Vec<_>>();
                assert_close(&numbers_of(&output, link.get(b"QuadPoints").unwrap()), &quads, &what);
                assert_eq!(annots[1].as_reference().unwrap(), widget, "{what}: the field is no longer the AcroForm's widget");
                let fields = output.get_dictionary(output.get_dictionary(output.trailer.get(b"Root").unwrap().as_reference().unwrap()).unwrap().get(b"AcroForm").unwrap().as_reference().unwrap()).unwrap();
                assert_eq!(fields.get(b"Fields").unwrap().as_array().unwrap(), &vec![Object::Reference(widget)], "{what}");
                let field = output.get_dictionary(widget).unwrap();
                assert_eq!(field.get(b"Subtype").unwrap().as_name().unwrap(), b"Widget");
                assert_eq!(field.get(b"FT").unwrap().as_name().unwrap(), b"Tx");
                assert_close(&numbers_of(&output, field.get(b"Rect").unwrap()), &rect([100., 40., 160., 80.]), &what);
                // The appearance is drawn through the same flip, from a copy of the original stream.
                let normal = field.get(b"AP").unwrap().as_dict().unwrap().get(b"N").unwrap().as_reference().unwrap();
                assert_ne!(normal, appearance, "{what}: the shared appearance stream was edited in place");
                let matrix = output.get_object(normal).unwrap().as_stream().unwrap().dict.get(b"Matrix").unwrap().clone();
                let (o, ex, ey) = (place((0., 0.)), place((1., 0.)), place((0., 1.)));
                assert_close(&numbers_of(&output, &matrix), &[Some(ex.0 - o.0), Some(ex.1 - o.1), Some(ey.0 - o.0), Some(ey.1 - o.1), Some(0.), Some(0.)], &what);
                // The page's own turn is now drawn into its content, so the field no longer undoes it.
                let turn = field.get(b"MK").unwrap().as_dict().unwrap().get(b"R").unwrap().as_i64().unwrap();
                assert_eq!(turn, if rotated { 0 } else { 90 }, "{what}");

                // Every destination to the page lands on the mirrored position of what it pointed at. A fit
                // to one edge keeps that edge, so on the turned source a /FitH becomes a /FitV.
                let (x, y) = place((50., 250.));
                assert_eq!(destination_of(&output, link.get(b"Dest").unwrap()), (b"XYZ".to_vec(), vec![Some(x), Some(y), Some(0.)]), "{what}: link");
                let catalog = output.get_dictionary(output.trailer.get(b"Root").unwrap().as_reference().unwrap()).unwrap();
                let bookmark = output.get_dictionary(output.get_dictionary(catalog.get(b"Outlines").unwrap().as_reference().unwrap()).unwrap().get(b"First").unwrap().as_reference().unwrap()).unwrap();
                let (name, coordinates) = destination_of(&output, bookmark.get(b"Dest").unwrap());
                let (x, y) = place((40., 200.));
                assert_eq!(name, b"XYZ"); assert_close(&coordinates, &[Some(x), Some(y), None], &format!("{what}: bookmark"));
                let tree = output.get_dictionary(catalog.get(b"Names").unwrap().as_reference().unwrap()).unwrap().get(b"Dests").unwrap().as_dict().unwrap();
                let (name, coordinates) = destination_of(&output, &tree.get(b"Names").unwrap().as_array().unwrap()[1]);
                if rotated { assert_eq!(name, b"FitV"); assert_close(&coordinates, &[Some(place((0., 250.)).0)], &what); }
                else { assert_eq!(name, b"FitH"); assert_close(&coordinates, &[Some(250.)], &what); }
                let side = output.get_dictionary(catalog.get(b"Dests").unwrap().as_reference().unwrap()).unwrap().get(b"side").unwrap().clone();
                let (name, coordinates) = destination_of(&output, &side);
                if rotated { assert_eq!(name, b"FitH"); assert_close(&coordinates, &[Some(place((60., 0.)).1)], &what); }
                else { assert_eq!(name, b"FitV"); assert_close(&coordinates, &[Some(140.)], &what); }
                let (name, coordinates) = destination_of(&output, catalog.get(b"OpenAction").unwrap());
                assert_eq!(name, if rotated { b"FitBV".to_vec() } else { b"FitBH".to_vec() }, "{what}");
                assert_close(&coordinates, &[Some(if rotated { place((0., 250.)).0 } else { 250. })], &what);
                let goto = output.get_dictionary(output.get_dictionary(pages[1]).unwrap().get(b"Annots").unwrap().as_array().unwrap()[0].as_reference().unwrap()).unwrap();
                let (name, coordinates) = destination_of(&output, goto.get(b"A").unwrap().as_dict().unwrap().get(b"D").unwrap());
                assert_eq!(name, b"FitR"); assert_close(&coordinates, &rect([30., 40., 90., 80.]), &format!("{what}: GoTo"));
                // The unmirrored page and its own annotation are untouched.
                assert_close(&numbers_of(&output, goto.get(b"Rect").unwrap()), &[Some(10.), Some(10.), Some(60.), Some(30.)], &what);
            }
        }
    }

    #[test]
    fn mirroring_keeps_destinations_that_target_other_pages_and_scene_structures() {
        let temp = TempDir::new().unwrap();
        let input = temp.0.join("linked.pdf");
        six_page_linked_form_fixture(&input);
        let original = fs::read(&input).unwrap();
        let source = Document::load(&input).unwrap();
        let pages: Vec<_> = source.get_pages().values().copied().collect();
        let scene = PdfScene { pages: (0..6).map(|index| oriented_page(index, (612., 792.), if index == 2 { 90 } else { 0 }, index == 1 || index == 2)).collect() };
        let result = export(&ExportRequest { paths: vec![input.clone()], scene, output_location: OutputLocation::AlongsideInput }, input.clone());
        assert!(result.error().is_none(), "{:?}", result.error());
        assert_eq!(fs::read(&input).unwrap(), original);
        let output = Document::load(&result.output_paths()[0]).unwrap();
        assert_eq!(output.get_pages().values().copied().collect::<Vec<_>>(), pages);
        for (index, page) in pages.iter().enumerate() {
            let annots = output.get_dictionary(*page).unwrap().get(b"Annots").unwrap().as_array().unwrap();
            assert_eq!(annots.len(), 3, "page {index}");
            let link = output.get_dictionary(annots[1].as_reference().unwrap()).unwrap();
            // /Fit has no coordinates, so it is the same whichever page it targets.
            assert_eq!(link.get(b"Dest").unwrap().as_array().unwrap(), &vec![Object::Reference(pages[(index + 1) % 6]), Object::Name(b"Fit".to_vec())]);
            let expected = if index == 1 || index == 2 { [352., 40., 442., 80.] } else { [170., 40., 260., 80.] };
            let widget = output.get_dictionary(annots[2].as_reference().unwrap()).unwrap();
            assert_close(&numbers_of(&output, widget.get(b"Rect").unwrap()), &expected.map(Some), &format!("page {index}"));
        }
        let catalog = output.get_dictionary(output.trailer.get(b"Root").unwrap().as_reference().unwrap()).unwrap();
        assert_eq!(output.get_dictionary(catalog.get(b"AcroForm").unwrap().as_reference().unwrap()).unwrap().get(b"Fields").unwrap().as_array().unwrap().len(), 6);
    }

    #[test]
    fn scene_rejects_mirroring_a_page_whose_annotations_another_page_shares_before_output() {
        let temp = TempDir::new().unwrap();
        let input = temp.0.join("shared.pdf");
        fixture(&input);
        let mut source = Document::load(&input).unwrap();
        let pages: Vec<_> = source.get_pages().values().copied().collect();
        let note = source.add_object(dictionary! {"Type"=>"Annot", "Subtype"=>"Text", "Rect"=>array(&[30., 40., 90., 80.])});
        for page in &pages { source.get_dictionary_mut(*page).unwrap().set("Annots", vec![Object::Reference(note)]); }
        source.save(&input).unwrap();
        let original = fs::read(&input).unwrap();
        let scene = PdfScene { pages: vec![oriented_page(0, (300., 200.), 0, false), oriented_page(1, (612., 792.), 0, true)] };
        let result = export(&ExportRequest { paths: vec![input.clone()], scene, output_location: OutputLocation::AlongsideInput }, input.clone());
        assert!(result.error().is_some_and(|error| error.message.contains("shares annotations")), "{:?}", result.error());
        assert!(result.output_paths().is_empty());
        assert!(!input.with_file_name("shared-edited-1.pdf").exists());
        assert_eq!(fs::read(&input).unwrap(), original);
    }

    #[test]
    fn scene_still_rejects_cropping_a_mirrored_page_with_annotations_or_destinations_before_output() {
        let temp = TempDir::new().unwrap();
        let input = temp.0.join("annotated.pdf");
        mirror_navigation_fixture(&input, false);
        let mut page = oriented_page(0, (200., 300.), 0, true);
        page.crop = Some(Rect { x: 0., y: 0., width: 100., height: 100. });
        let scene = PdfScene { pages: vec![page, oriented_page(1, (200., 300.), 0, false)] };
        let result = export(&ExportRequest { paths: vec![input.clone()], scene, output_location: OutputLocation::AlongsideInput }, input.clone());
        assert!(result.error().is_some_and(|error| error.message.contains("annotations or form widgets") && error.message.contains("crop")), "{:?}", result.error());
        assert!(result.output_paths().is_empty());
        assert!(!input.with_file_name("annotated-edited-1.pdf").exists());
    }

    #[test]
    fn scene_rejects_nested_page_tree_before_output() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("nested.pdf"); nested_fixture(&input);
        let scene=PdfScene { pages:vec![ScenePage { source_index:Some(0), width:300., height:200., rotation:0, mirrored: false, crop:None, source_rotation:None, source_box:None, objects:vec![] }, ScenePage { source_index:Some(1), width:612., height:792., rotation:0, mirrored: false, crop:None, source_rotation:None, source_box:None, objects:vec![] }] };
        let result=export(&ExportRequest {paths:vec![input.clone()],scene,output_location:OutputLocation::AlongsideInput},input.clone());
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
        let scene=PdfScene { pages:vec![ScenePage { source_index:Some(0), width:300., height:200., rotation:0, mirrored: false, crop:Some(Rect{x:0.,y:0.,width:100.,height:100.}), source_rotation:None, source_box:None, objects:vec![] }] };
        let result=export(&ExportRequest {paths:vec![input.clone()],scene,output_location:OutputLocation::AlongsideInput},input.clone());
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
            ScenePage {source_index:Some(0),width:300.,height:200.,rotation:0, mirrored: false,crop:None,source_rotation:None,source_box:None,objects:vec![]},
            ScenePage {source_index:Some(1),width:612.,height:792.,rotation:0, mirrored: false,crop:None,source_rotation:None,source_box:None,objects:vec![]},
        ] };
        let result=export(&ExportRequest {paths:vec![input.clone()],scene,output_location:OutputLocation::AlongsideInput},input.clone());
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
                ScenePage {source_index:Some(0),width:300.,height:200.,rotation:0, mirrored: false,crop:None,source_rotation:None,source_box:None,objects:vec![]},
                ScenePage {source_index:Some(1),width:612.,height:792.,rotation:0, mirrored: false,crop:None,source_rotation:None,source_box:None,objects:vec![]},
            ] };
            let result=export(&ExportRequest {paths:vec![input.clone()],scene,output_location:OutputLocation::AlongsideInput},input.clone());
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
            ScenePage {source_index:Some(0),width:300.,height:200.,rotation:0, mirrored: false,crop:Some(Rect{x:0.,y:0.,width:100.,height:100.}),source_rotation:None,source_box:None,objects:vec![]},
            ScenePage {source_index:Some(1),width:612.,height:792.,rotation:0, mirrored: false,crop:None,source_rotation:None,source_box:None,objects:vec![]},
        ] };
        let result=export(&ExportRequest {paths:vec![input.clone()],scene,output_location:OutputLocation::AlongsideInput},input.clone());
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
            ScenePage {source_index:Some(0),width:200.,height:300.,rotation:0, mirrored: false,crop:None,source_rotation:None,source_box:None,objects:vec![]},
            ScenePage {source_index:Some(1),width:612.,height:792.,rotation:0, mirrored: false,crop:None,source_rotation:None,source_box:None,objects:vec![]},
        ] };
        let result=export(&ExportRequest {paths:vec![input.clone()],scene,output_location:OutputLocation::AlongsideInput},input.clone());
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
        let scene=PdfScene { pages:vec![ScenePage {source_index:Some(1),width:612.,height:792.,rotation:0, mirrored: false,crop:None,source_rotation:None,source_box:None,objects:vec![]}] };
        let result=export(&ExportRequest {paths:vec![input.clone()],scene,output_location:OutputLocation::AlongsideInput},input.clone());
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
            ScenePage { source_index: Some(0), width: 300., height: 200., rotation: 0, mirrored: false, crop: None, source_rotation: Some(90), source_box: Some(Rect { x: 20., y: 30., width: 200., height: 300. }), objects: vec![] },
            ScenePage { source_index: Some(1), width: 612., height: 792., rotation: 0, mirrored: false, crop: None, source_rotation: Some(0), source_box: Some(Rect { x: 0., y: 0., width: 612., height: 792. }), objects: vec![] },
        ] };
        assert!(compose(&input, &identity).is_ok());
        let mut overlay = identity;
        overlay.pages[0].objects.push(object(Kind::Text));
        let result = export(&ExportRequest { paths: vec![input.clone()], scene: overlay, output_location: OutputLocation::AlongsideInput }, input.clone());
        assert!(result.error().is_some_and(|error| error.message.contains("tagged PDF")), "{result:?}");
        assert!(result.output_paths().is_empty());
        assert!(!input.with_file_name("tagged-overlay-edited-1.pdf").exists());
    }

    fn many_page_fixture(path: &Path, pages: usize) {
        let mut d = Document::with_version("1.7");
        let root = d.new_object_id();
        let content = d.add_object(Stream::new(dictionary!{}, b"q 0 0 1 rg 40 60 50 70 re f Q".to_vec()));
        let kids = (0..pages).map(|_| Object::Reference(d.add_object(dictionary! {"Type"=>"Page", "Parent"=>root, "MediaBox"=>array(&[0.,0.,612.,792.]), "Contents"=>content}))).collect::<Vec<_>>();
        d.objects.insert(root, Object::Dictionary(dictionary! {"Type"=>"Pages", "Kids"=>kids, "Count"=>pages as i64}));
        let catalog = d.add_object(dictionary! {"Type"=>"Catalog","Pages"=>root});
        d.trailer.set("Root",catalog);
        d.save(path).unwrap();
    }

    fn with_renderer_override<T>(renderer: &Path, run: impl FnOnce() -> T) -> T {
        let previous = std::env::var_os("TOOLBOX_PDFTOPPM_PATH");
        std::env::set_var("TOOLBOX_PDFTOPPM_PATH", renderer);
        let result = run();
        match previous {
            Some(value) => std::env::set_var("TOOLBOX_PDFTOPPM_PATH", value),
            None => std::env::remove_var("TOOLBOX_PDFTOPPM_PATH"),
        }
        result
    }

    #[test]
    fn source_thumbnail_deadlines_are_recorded_per_file_plus_per_page() {
        assert_eq!(super::super::PDF_THUMBNAIL_MAX_DIMENSION, 320);
        assert_eq!(super::super::PDF_THUMBNAIL_MAX_BATCH, 12);
        assert_eq!(super::super::PDF_THUMBNAIL_FILE_TIMEOUT, std::time::Duration::from_secs(5));
        assert_eq!(super::super::PDF_THUMBNAIL_PAGE_TIMEOUT, std::time::Duration::from_secs(3));
        let start = Instant::now();
        assert_eq!(thumbnail_deadline(start, 1) - start, std::time::Duration::from_secs(8));
        assert_eq!(thumbnail_deadline(start, 4) - start, std::time::Duration::from_secs(17));
    }

    #[test]
    fn source_thumbnails_reject_empty_duplicate_out_of_range_and_oversized_sets() {
        let temp = TempDir::new().unwrap();
        let input = temp.0.join("source.pdf");
        many_page_fixture(&input, 20);
        let cases = [
            (vec![], "Thumbnail requests must include 1–12 page indices"),
            (vec![3, 3], "Thumbnail page indices must be unique"),
            (vec![20], "Thumbnail page outside document"),
            ((0..13).collect(), "Thumbnail requests must include 1–12 page indices"),
        ];
        for (page_indices, message) in cases {
            let error = source_thumbnails(&SourceThumbnailsRequest { path: input.clone(), page_indices }).err().expect("request should be rejected");
            assert!(matches!(error.kind, crate::kit::contracts::ErrorKind::InvalidInput), "kind: {:?}", error.kind);
            assert_eq!(error.message, message);
        }
        let missing = source_thumbnails(&SourceThumbnailsRequest { path: temp.0.join("missing.pdf"), page_indices: vec![0] }).err().unwrap();
        assert_eq!(missing.message, UNREADABLE_PDF_MESSAGE);
    }

    #[cfg(unix)]
    #[test]
    fn source_thumbnails_isolate_a_failing_page_and_never_reuse_another_pages_image() {
        use std::os::unix::fs::PermissionsExt;
        let _guard = crate::kit::lock_process_env();
        let temp = TempDir::new().unwrap();
        let input = temp.0.join("source.pdf");
        many_page_fixture(&input, 5);
        let png = temp.0.join("page.png");
        image::RgbImage::from_pixel(232, 300, image::Rgb([200, 200, 255])).save(&png).unwrap();
        // pdftoppm -png -singlefile -scale-to N -f P -l P input prefix: page 2 fails, page 4 "succeeds" without writing an image.
        let fake = temp.0.join("pdftoppm");
        fs::write(&fake, format!("#!/bin/sh\ncase \"$6\" in 2) echo broken >&2; exit 1;; 4) exit 0;; esac\nexec cp '{}' \"${{10}}.png\"\n", png.display())).unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        let before = fs::read(&input).unwrap();
        let results = with_renderer_override(&fake, || source_thumbnails(&SourceThumbnailsRequest { path: input.clone(), page_indices: vec![0, 1, 2, 3] })).unwrap();
        assert_eq!(results.iter().map(|result| result.page_index).collect::<Vec<_>>(), vec![0, 1, 2, 3]);
        for index in [0, 2] {
            let thumbnail = results[index].thumbnail.as_ref().expect("healthy page should have a thumbnail");
            assert!(thumbnail.data_url.starts_with("data:image/png;base64,"));
            assert_eq!((thumbnail.width, thumbnail.height), (232, 300));
            assert!(results[index].error.is_none());
        }
        assert!(results[1].thumbnail.is_none() && results[1].error.is_some(), "page 2 should fail alone");
        assert!(results[3].thumbnail.is_none() && results[3].error.is_some(), "page 4 wrote nothing, so it must not reuse another page's image");
        assert_eq!(fs::read(&input).unwrap(), before);
    }

    #[test]
    fn source_thumbnails_render_requested_source_pages_with_the_bundled_renderer() {
        let _guard = crate::kit::lock_process_env();
        if let Err(e) = renderer() { if std::env::var_os("TOOLBOX_REQUIRE_PDF_RENDERER").is_some() { panic!("{e}") }; eprintln!("Renderer unavailable: {e}"); return; }
        let temp = TempDir::new().unwrap();
        let input = temp.0.join("source.pdf");
        fixture(&input);
        let results = source_thumbnails(&SourceThumbnailsRequest { path: input.clone(), page_indices: vec![1, 0] }).unwrap();
        assert_eq!(results.iter().map(|result| result.page_index).collect::<Vec<_>>(), vec![1, 0]);
        let sizes = results.iter().map(|result| {
            let thumbnail = result.thumbnail.as_ref().unwrap_or_else(|| panic!("page {} failed: {:?}", result.page_index, result.error));
            assert!(thumbnail.width.max(thumbnail.height) <= super::super::PDF_THUMBNAIL_MAX_DIMENSION);
            (thumbnail.width, thumbnail.height)
        }).collect::<Vec<_>>();
        assert!(sizes[0].1 > sizes[0].0, "the portrait source page stays portrait");
        assert!(sizes[1].0 > sizes[1].1, "the source /Rotate 90 page renders landscape");
    }

    #[test]
    fn inspect_refuses_documents_over_the_recorded_limits_with_limit_exceeded() {
        use crate::kit::contracts::{ErrorKind, LimitKind, OutcomePointer};
        let limits = crate::kit::common::limits::recorded("export_pdf_scene").unwrap();
        assert_eq!(limits.max_pages, Some(5_000));
        assert_eq!(limits.max_bytes, 1_000_000_000);
        let temp = TempDir::new().unwrap();

        let at_limit = temp.0.join("at-limit.pdf");
        many_page_fixture(&at_limit, 5_000);
        assert_eq!(inspect(&at_limit).unwrap().pages.len(), 5_000);

        let over = temp.0.join("archive.pdf");
        many_page_fixture(&over, 5_001);
        let error = inspect(&over).expect_err("over the page limit");
        assert_eq!(error.kind, ErrorKind::LimitExceeded { limit: LimitKind::PageCount });
        assert_eq!(error.message, "It has 5,001 pages. Toolbox works with up to 5,000 pages in one PDF. Use Split to make smaller parts, then open each one.");
        assert_eq!(error.pointer, Some(OutcomePointer::Tool { tool_id: "pdf-split".into() }));

        let large = temp.0.join("large.pdf");
        fs::File::create(&large).unwrap().set_len(limits.max_bytes + 1).unwrap();
        let error = inspect(&large).expect_err("over the size limit");
        assert_eq!(error.kind, ErrorKind::LimitExceeded { limit: LimitKind::FileSize });
        assert_eq!(error.message, "Larger than the 1 GB limit for one PDF. Reduce its size and try again.");
    }
}
