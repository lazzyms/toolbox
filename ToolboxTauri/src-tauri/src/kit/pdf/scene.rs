use std::{collections::{HashMap, HashSet}, fs::{self, OpenOptions}, io::Write, path::{Path, PathBuf}, process::Command, sync::atomic::{AtomicU64, Ordering}};
use base64::Engine;
use flate2::{write::ZlibEncoder, Compression};
use lopdf::{dictionary, Document, Object, ObjectId, Stream};
use quick_xml::{events::Event, escape::unescape, Reader as XmlReader, XmlVersion};
use serde::{Deserialize, Serialize};
use crate::kit::{common::{JobOutcome, OutputLocation, OutputNaming}, contracts::ToolError};
use super::metadata::{PdfDocumentMetadata, PdfPageMetadata, PdfTextRun};

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
    pub rotation: i32, pub crop: Option<Rect>, pub objects: Vec<SceneObject>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct PdfScene { pub pages: Vec<ScenePage> }
#[derive(Deserialize)]
#[serde(rename_all="camelCase")]
pub struct PreviewRequest { pub path: PathBuf, pub scene: PdfScene, pub page_index: usize }
#[derive(Deserialize)]
#[serde(rename_all="camelCase")]
pub struct ExportRequest { pub paths: Vec<PathBuf>, pub scene: PdfScene, pub output_location: OutputLocation }
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct ScenePreview { pub data_url: String, pub width: u32, pub height: u32 }

fn err(e: impl std::fmt::Display) -> String { e.to_string() }
fn resolve<'a>(doc: &'a Document, mut value: &'a Object) -> Result<&'a Object, String> {
    let mut seen = HashSet::new();
    while let Object::Reference(id) = value {
        if !seen.insert(*id) { return Err("Cyclic PDF reference".into()); }
        value = doc.get_object(*id).map_err(err)?;
    }
    Ok(value)
}
fn inherited(doc: &Document, mut id: ObjectId, key: &[u8]) -> Result<Option<Object>, String> {
    let mut seen = HashSet::new();
    loop {
        if !seen.insert(id) { return Err("Cyclic PDF page tree".into()); }
        let d = doc.get_dictionary(id).map_err(err)?;
        if let Ok(v) = d.get(key) { return Ok(Some(resolve(doc, v)?.clone())); }
        match d.get(b"Parent") { Ok(v) => id = v.as_reference().map_err(err)?, Err(_) => return Ok(None) }
    }
}
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

fn watermark_placements(area: &Rect, font_size: f32, text: &str, pattern: Option<&WatermarkPattern>) -> Vec<(f32, f32, f32)> {
    let pattern = pattern.unwrap_or(&WatermarkPattern::AcrossPage);
    let text_width = text.chars().count().max(1) as f32 * font_size * 0.6;
    let center_x = area.x + area.width / 2.;
    let center_y = area.y + area.height / 2.;
    match pattern {
        WatermarkPattern::AcrossPage => {
            let step_x = (text_width + font_size * 2.).max(100.);
            let step_y = (font_size * 4.).max(80.);
            let mut placements = Vec::new();
            let mut y = area.y - area.height;
            while y <= area.y + area.height * 2. {
                let offset = ((y - area.y) / step_y).round() * step_x * 0.45;
                let mut x = area.x - area.width + offset;
                while x <= area.x + area.width * 2. {
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

fn catalog_and_pages_root(document: &Document) -> Result<(ObjectId, ObjectId), String> {
    let catalog = document.trailer.get(b"Root").map_err(err)?.as_reference().map_err(err)?;
    let catalog_dict = document.get_dictionary(catalog).map_err(err)?;
    let pages = catalog_dict.get(b"Pages").map_err(err)?.as_reference().map_err(err)?;
    document.get_dictionary(pages).map_err(err)?;
    Ok((catalog, pages))
}

fn has_catalog_structure(document: &Document, catalog: ObjectId, key: &[u8]) -> Result<bool, String> {
    Ok(document.get_dictionary(catalog).map_err(err)?.get(key).is_ok())
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

fn field_has_signature(document: &Document, value: &Object, seen: &mut HashSet<ObjectId>) -> Result<bool, String> {
    let id = value.as_reference().map_err(err)?;
    if !seen.insert(id) { return Err("Cyclic PDF form field tree".into()); }
    let field = document.get_dictionary(id).map_err(err)?;
    if field.get(b"FT").ok().and_then(|value| value.as_name().ok()) == Some(b"Sig") { return Ok(true); }
    let Some(kids) = field.get(b"Kids").ok() else { return Ok(false); };
    resolve(document, kids)?.as_array().map_err(err)?.iter().map(|kid| field_has_signature(document, kid, seen)).collect::<Result<Vec<_>, String>>().map(|values| values.into_iter().any(|value| value))
}

fn document_has_signature(document: &Document, catalog: ObjectId, pages: &[ObjectId]) -> Result<bool, String> {
    if let Ok(acro_form) = document.get_dictionary(catalog).map_err(err)?.get(b"AcroForm") {
        let acro_form = resolve(document, acro_form)?.as_dict().map_err(err)?;
        if acro_form.get(b"SigFlags").ok().and_then(|value| value.as_i64().ok()).is_some_and(|flags| flags != 0) { return Ok(true); }
        if let Some(fields) = acro_form.get(b"Fields").ok() {
            let mut seen = HashSet::new();
            if resolve(document, fields)?.as_array().map_err(err)?.iter().map(|field| field_has_signature(document, field, &mut seen)).collect::<Result<Vec<_>, String>>()?.into_iter().any(|value| value) { return Ok(true); }
        }
    }
    pages.iter().map(|page| {
        let Some(annotations) = document.get_dictionary(*page).map_err(err)?.get(b"Annots").ok() else { return Ok(false); };
        resolve(document, annotations)?.as_array().map_err(err)?.iter().map(|annotation| {
            let id = annotation.as_reference().map_err(err)?;
            Ok(document.get_dictionary(id).map_err(err)?.get(b"FT").ok().and_then(|value| value.as_name().ok()) == Some(b"Sig"))
        }).collect::<Result<Vec<_>, String>>().map(|values| values.into_iter().any(|value| value))
    }).collect::<Result<Vec<_>, String>>().map(|values| values.into_iter().any(|value| value))
}

fn scene_preflight(document: &Document, scene: &PdfScene, source_pages: &[ObjectId]) -> Result<ScenePreflight, String> {
    let (catalog, pages_root) = catalog_and_pages_root(document)?;
    if document_has_signature(document, catalog, source_pages)? {
        return Err("Digitally signed PDFs cannot be edited because scene export would invalidate the signature; remove the signature or use an unsigned copy".into());
    }
    let root = document.get_dictionary(pages_root).map_err(err)?;
    if root.get(b"Type").map_err(err)?.as_name().map_err(err)? != b"Pages" {
        return Err("PDF catalog has an unsupported page-tree root".into());
    }
    let kids = root.get(b"Kids").map_err(err)?.as_array().map_err(err)?;
    if kids.len() != source_pages.len() {
        return Err("PDF uses a nested or unsupported page tree; scene export was rejected before output".into());
    }
    for (index, kid) in kids.iter().enumerate() {
        let page_id = kid.as_reference().map_err(err)?;
        let page = document.get_dictionary(page_id).map_err(err)?;
        if page.get(b"Type").map_err(err)?.as_name().map_err(err)? != b"Page" || page_id != source_pages[index] || page.get(b"Parent").map_err(err)?.as_reference().map_err(err)? != pages_root {
            return Err("PDF uses a nested or unsupported page tree; scene export was rejected before output".into());
        }
    }

    let mut seen = HashSet::new();
    let mut retained = HashSet::new();
    for page in &scene.pages {
        if let Some(index) = page.source_index {
            let source = *source_pages.get(index).ok_or("Scene source page is outside the document")?;
            if !seen.insert(source) { return Err("A source page can appear only once in a scene".into()); }
            retained.insert(source);
        }
    }
    let has_navigation = [b"Outlines".as_slice(), b"Names", b"Dests", b"PageLabels", b"OpenAction"]
        .into_iter()
        .map(|key| has_catalog_structure(document, catalog, key))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .any(|present| present);
    if retained.len() != source_pages.len() && (has_navigation || has_catalog_structure(document, catalog, b"AcroForm")?) {
        return Err("This PDF contains navigation or form structures that could target a removed page; scene export was rejected before output".into());
    }
    for page in &scene.pages {
        if let Some(index) = page.source_index {
            let page_id = source_pages[index];
            let page_dict = document.get_dictionary(page_id).map_err(err)?;
            let has_annotations = page_dict.get(b"Annots").is_ok();
            let has_widget = page_has_widget(document, page_id)?;
            let source_rotation = inherited(document, page_id, b"Rotate")?.map(|value| value.as_i64().map_err(err)).transpose()?.unwrap_or(0).rem_euclid(360) as i32;
            let geometry = geometry(document, page_id)?;
            let output_crop = page.crop.as_ref().map(|crop| (crop.width, crop.height)).unwrap_or((page.width, page.height));
            let source_box_matches_output = geometry.bbox[0].abs() <= 0.1 && geometry.bbox[1].abs() <= 0.1 && (geometry.bbox[2] - geometry.bbox[0] - output_crop.0).abs() <= 0.1 && (geometry.bbox[3] - geometry.bbox[1] - output_crop.1).abs() <= 0.1;
            let changes_page_coordinates = page.crop.is_some() || !source_box_matches_output || (page.width - geometry.width).abs() > 0.1 || (page.height - geometry.height).abs() > 0.1 || page.rotation.rem_euclid(360) != source_rotation;
            if has_annotations && changes_page_coordinates {
                return Err("This PDF page has annotations or form widgets and the requested crop or rotation would change their coordinates; scene export was rejected before output".into());
            }
            if (page.crop.is_some() || changes_page_coordinates) && has_widget {
                return Err("Cropping or rotating a PDF with form widgets is unsupported; remove the page operation or edit the source PDF first".into());
            }
        }
    }
    Ok(ScenePreflight { pages_root, source_pages: source_pages.to_vec() })
}

fn scene_resources(document: &Document, page_id: ObjectId, xobjects: lopdf::Dictionary, fonts: &HashMap<&'static str, ObjectId>, states: lopdf::Dictionary) -> Result<lopdf::Dictionary, String> {
    let mut resources = match inherited(document, page_id, b"Resources")? {
        Some(value) => value.as_dict().map_err(err)?.clone(),
        None => lopdf::Dictionary::new(),
    };
    let mut page_xobjects = resources.get(b"XObject").ok().map(|value| resolve(document, value).and_then(|value| value.as_dict().map_err(err).map(Clone::clone))).transpose()?.unwrap_or_default();
    for (name, value) in xobjects.into_iter() { page_xobjects.set(name, value); }
    resources.set("XObject", page_xobjects);
    let mut page_fonts = resources.get(b"Font").ok().map(|value| resolve(document, value).and_then(|value| value.as_dict().map_err(err).map(Clone::clone))).transpose()?.unwrap_or_default();
    for (name, value) in fonts { page_fonts.set(*name, *value); }
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
    let preflight = scene_preflight(&doc, scene, &sources)?;
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
        let mut xobjects=lopdf::Dictionary::new();
        let mut content=format!("q\n1 0 0 -1 {} {} cm\n0 0 {} {} re W n\n",-crop.x,crop.y+crop.height,page.width,page.height);
        if let Some(index)=page.source_index {
            let id=*sources.get(index).ok_or("Source page index outside document")?;
            let g=geometry(&doc,id)?;
            if (page.width-g.width).abs()>0.1 || (page.height-g.height).abs()>0.1 { return Err("Scene dimensions do not match source page".into()); }
            let resources=inherited(&doc,id,b"Resources")?.unwrap_or(Object::Dictionary(dictionary!{}));
            resources.as_dict().map_err(err)?;
            let mut form=dictionary!{"Type"=>"XObject","Subtype"=>"Form","BBox"=>array(&g.bbox),"Resources"=>resources};
            if let Ok(group)=doc.get_dictionary(id).map_err(err)?.get(b"Group") { form.set("Group",group.clone()); }
            let bytes=doc.get_page_content(id);
            let form_id=doc.add_object(Stream::new(form,bytes));
            xobjects.set("Original",form_id);
            content.push_str(&format!("q\n{}/Original Do\nQ\n",cm(g.matrix)));
        }
        if page.objects.len()>10000 { return Err("Too many scene objects".into()); }
        let mut states=lopdf::Dictionary::new();
        for (i,o) in page.objects.iter().enumerate() {
            valid_rect(&o.rect)?;
            if !o.opacity.is_finite() || !(0. ..=1.).contains(&o.opacity) { return Err("Opacity must be between 0 and 1".into()); }
            if !o.font_size.is_finite() || o.font_size<=0. || o.font_size>1000. { return Err("Invalid font size".into()); }
            if o.highlight_mode.is_some() && !matches!(&o.kind, Kind::Highlight) { return Err("Highlight mode is only valid for highlight objects".into()); }
            let rgb=rgb(&o.color)?;
            states.set(format!("S{i}"),dictionary!{"Type"=>"ExtGState","ca"=>o.opacity,"CA"=>o.opacity});
            let r=&o.rect;
            content.push_str(&format!("q /S{i} gs {} {} {} rg {} {} {} RG 2 w 1 J 1 j\n",rgb[0],rgb[1],rgb[2],rgb[0],rgb[1],rgb[2]));
            match o.kind {
                Kind::Highlight=>content.push_str(&format!("{} {} {} {} re f\n",r.x,r.y,r.width,r.height)),
                Kind::Shape=>content.push_str(&shape_content(o.shape.as_ref(), r)),
                Kind::Signature=>{
                    match o.signature_mode {
                        Some(SignatureMode::Image) => {
                            let path=o.signature_path.as_ref().ok_or("Choose a signature image before placing it")?;
                            let name=format!("Sig{i}");
                            add_signature_image(&mut doc, &mut xobjects, &name, signature_image(path)?);
                            content.push_str(&format!("q {} 0 0 {} {} {} /{name} Do Q\n",r.width,r.height,r.x,r.y));
                        }
                        Some(SignatureMode::Text) => {
                            let (_, resource)=font_spec(o.font_family.as_deref())?;
                            let encoded=encode_text(&o.text)?;
                            content.push_str(&format!("BT /{resource} {} Tf 1 0 0 -1 {} {} Tm <{encoded}> Tj ET\n",o.font_size,r.x,r.y+o.font_size));
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
                    let encoded_lines=o.text.lines().map(encode_text).collect::<Result<Vec<_>,_>>()?;
                    content.push_str(&format!("{} {} {} {} re W n\n",r.x,r.y,r.width,r.height));
                    for (line,encoded) in encoded_lines.iter().enumerate() {
                        content.push_str(&format!("BT /{resource} {} Tf 1 0 0 -1 {} {} Tm <{}> Tj ET\n",o.font_size,r.x,r.y+o.font_size*(1.+line as f32*1.2),encoded));
                    }
                },
                Kind::Watermark=>{
                    let (_, resource)=font_spec(o.font_family.as_deref())?;
                    let encoded=encode_text(&o.text)?;
                    let area=page.crop.as_ref().cloned().unwrap_or(Rect{x:0.,y:0.,width:page.width,height:page.height});
                    for (x,y,angle) in watermark_placements(&area,o.font_size,&o.text,o.watermark_pattern.as_ref()) {
                        let radians=angle.to_radians(); let c=radians.cos(); let s=radians.sin();
                        content.push_str(&format!("BT /{resource} {} Tf {} {} {} {} {} {} Tm <{encoded}> Tj ET\n",o.font_size,c,s,s,-c,x,y));
                    }
                },
            }
            content.push_str("Q\n");
        }
        content.push_str("Q\n");
        let stream=doc.add_object(Stream::new(dictionary!{},content.into_bytes()));
        let mut page_fonts=lopdf::Dictionary::new();
        for (resource,id) in &fonts { page_fonts.set(*resource,*id); }
        let source_index = page.source_index;
        if let Some(index) = source_index {
            let id = preflight.source_pages[index];
            let resources = scene_resources(&doc, id, xobjects, &fonts, states)?;
            let page_dict = doc.get_dictionary_mut(id).map_err(err)?;
            page_dict.set("MediaBox",array(&[0.,0.,crop.width,crop.height]));
            page_dict.set("CropBox",array(&[0.,0.,crop.width,crop.height]));
            page_dict.set("Rotate",page.rotation.rem_euclid(360));
            page_dict.set("Resources",resources);
            page_dict.set("Contents",stream);
            kids.push(Object::Reference(id));
        } else {
            let id=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>preflight.pages_root,"MediaBox"=>array(&[0.,0.,crop.width,crop.height]),"Rotate"=>page.rotation.rem_euclid(360),"Resources"=>dictionary!{"XObject"=>xobjects,"Font"=>page_fonts,"ExtGState"=>states},"Contents"=>stream});
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
    let path=std::env::var_os("TOOLBOX_PDFTOPPM_PATH").map(PathBuf::from).unwrap_or_else(||PathBuf::from("pdftoppm"));
    if Command::new(&path).arg("-h").output().map(|o|o.status.success()).unwrap_or(false) { Ok(path) } else { Err("pdftoppm is required for PDF previews. Set TOOLBOX_PDFTOPPM_PATH or add it to PATH.".into()) }
}

fn text_extractor()->Option<PathBuf> {
    std::env::var_os("TOOLBOX_PDFTOTEXT_PATH").map(PathBuf::from).filter(|path|path.is_file())
        .or_else(|| crate::kit::resources::application_resource_root().and_then(|root| [root.join("pdf-bin").join("pdftotext"), root.join("resources").join("pdftotext"), root.join("pdftotext")].into_iter().find(|path|path.is_file())))
        .or_else(|| Command::new("pdftotext").arg("-h").output().ok().filter(|output|output.status.success()).map(|_|PathBuf::from("pdftotext")))
}

fn xml_attribute(element: &quick_xml::events::BytesStart<'_>, name: &[u8]) -> Option<String> {
    element.attributes().flatten().find(|attribute|attribute.key.as_ref() == name).and_then(|attribute|attribute.normalized_value(XmlVersion::Implicit1_0).ok().map(|value|value.into_owned()))
}

fn xml_number(element: &quick_xml::events::BytesStart<'_>, name: &[u8]) -> Option<f32> { xml_attribute(element,name)?.parse().ok() }

fn extract_text_runs(path: &Path, page_number: usize, width: f32, height: f32) -> Option<Vec<PdfTextRun>> {
    let extractor=text_extractor()?;
    let output=Command::new(extractor).args(["-bbox-layout","-enc","UTF-8","-f"]).arg(page_number.to_string()).arg("-l").arg(page_number.to_string()).arg(path).arg("-").output().ok()?;
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
                    if !text.trim().is_empty() && runs.len() < 10_000 && x_max > x_min && y_max > y_min {
                        let sx=width/source_size.0.max(1.); let sy=height/source_size.1.max(1.);
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
    let output=Command::new(renderer).args(["-png","-singlefile","-scale-to","1600","-f"]).arg((index+1).to_string()).arg("-l").arg((index+1).to_string()).arg(path).arg(&prefix).output().map_err(err)?;
    if !output.status.success() { return Err("pdftoppm could not render this PDF".into()); }
    let bytes=fs::read(prefix.with_extension("png")).map_err(err)?;
    let image=image::load_from_memory_with_format(&bytes,image::ImageFormat::Png).map_err(err)?;
    if image.width()>1600 || image.height()>1600 { return Err("Preview exceeded size limit".into()); }
    Ok(ScenePreview{data_url:format!("data:image/png;base64,{}",base64::engine::general_purpose::STANDARD.encode(bytes)),width:image.width(),height:image.height()})
}
pub fn preview(request:&PreviewRequest)->Result<ScenePreview,String> {
    if request.page_index>=request.scene.pages.len() { return Err("Preview page outside scene".into()); }
    let renderer=renderer()?;
    let temp=TempDir::new()?;
    let path=temp.0.join("scene.pdf");
    compose(&request.path,&request.scene)?.save(&path).map_err(err)?;
    render(&path,request.page_index,&renderer,&temp)
}
pub fn inspect(path:&Path)->Result<PdfDocumentMetadata,String> {
    let doc=load(path)?;
    let mut pages=Vec::new();
    for (index,id) in doc.get_pages().values().enumerate() {
        let g=geometry(&doc,*id)?;
        pages.push(PdfPageMetadata{index,x:0.,y:0.,width:g.width,height:g.height,preview:None,text_runs:None});
    }
    if pages.is_empty() { return Err("PDF contains no pages".into()); }
    let scene=PdfScene{pages:pages.iter().map(|p|ScenePage{source_index:Some(p.index),width:p.width,height:p.height,rotation:0,crop:None,objects:vec![]}).collect()};
    let renderer=renderer()?;
    let temp=TempDir::new()?;
    let normalized=temp.0.join("scene.pdf");
    compose(path,&scene)?.save(&normalized).map_err(err)?;
    for p in &mut pages {
        p.preview=Some(render(&normalized,p.index,&renderer,&temp)?.data_url);
        p.text_runs=extract_text_runs(path,p.index+1,p.width,p.height);
    }
    Ok(PdfDocumentMetadata{path:path.to_path_buf(),pages})
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
    match result { Ok(path)=>JobOutcome{input_path:input,output_paths:vec![path],detail:"PDF scene exported".into(),failure:None},Err(e)=>JobOutcome::failure(input,ToolError::processing(e)) }
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
    fn object(kind: Kind) -> SceneObject {
        SceneObject { kind, rect: Rect{x:30.,y:40.,width:120.,height:40.}, text:"LOCAL".into(),font_size:18.,color:"#0066CC".into(),opacity:0.5,
            strokes:vec![vec![Point{x:0.,y:0.5},Point{x:0.5,y:0.},Point{x:1.,y:1.}]], shape:None, highlight_mode:None,
            signature_mode:None, signature_path:None, font_family:None, watermark_pattern:None }
    }
    fn scene() -> PdfScene {
        PdfScene { pages:vec![
            ScenePage {source_index:Some(1),width:612.,height:792.,rotation:180,crop:Some(Rect{x:10.,y:20.,width:500.,height:600.}),objects:vec![object(Kind::Highlight),object(Kind::Text),object(Kind::Shape),object(Kind::Watermark),object(Kind::Signature)]},
            ScenePage {source_index:None,width:300.,height:200.,rotation:90,crop:None,objects:vec![object(Kind::Text)]},
            ScenePage {source_index:Some(0),width:300.,height:200.,rotation:0,crop:None,objects:vec![]},
        ] }
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
    fn exports_preserve_original_and_existing_outputs_byte_for_byte() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); fixture(&input);
        let original=fs::read(&input).unwrap();
        let occupied=temp.0.join("source-edited-1.pdf"); fs::write(&occupied,b"existing output").unwrap();
        let request=ExportRequest {paths:vec![input.clone()],scene:scene(),output_location:OutputLocation::AlongsideInput};
        let a=export(&request,input.clone()); let b=export(&request,input.clone());
        assert!(a.failure.is_none(),"{:?}",a.failure); assert!(b.failure.is_none());
        assert_ne!(a.output_paths,b.output_paths); assert_ne!(a.output_paths[0],input);
        assert_eq!(fs::read(&input).unwrap(),original); assert_eq!(fs::read(occupied).unwrap(),b"existing output");
        assert_eq!(Document::load(&a.output_paths[0]).unwrap().get_pages().len(),3);
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
        assert!(result.failure.is_some()); assert!(result.output_paths.is_empty());
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(),1);
        assert!(compose(&input,&PdfScene{pages:vec![]}).is_err());
    }
    #[test]
    fn preview_and_export_are_pixel_identical_and_temp_storage_is_cleaned() {
        let renderer=match renderer() { Ok(r)=>r,Err(e)=>{if std::env::var_os("TOOLBOX_REQUIRE_PDF_RENDERER").is_some(){panic!("{e}")}; eprintln!("Renderer unavailable: {e}");return;} };
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); fixture(&input);
        let before=fs::read(&input).unwrap();
        let request=ExportRequest{paths:vec![input.clone()],scene:scene(),output_location:OutputLocation::AlongsideInput};
        let exported=export(&request,input.clone()); assert!(exported.failure.is_none());
        for index in 0..3 {
            let preview=preview(&PreviewRequest{path:input.clone(),scene:scene(),page_index:index}).unwrap();
            let actual=render(&exported.output_paths[0],index,&renderer,&temp).unwrap();
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
    fn extracts_selectable_text_runs_when_the_local_poppler_helper_is_available() {
        if text_extractor().is_none() { return; }
        let temp=TempDir::new().unwrap(); let input=temp.0.join("source.pdf"); fixture(&input);
        let runs=extract_text_runs(&input,1,300.,200.).unwrap();
        assert!(runs.iter().any(|run|run.text.contains("Original")),"runs: {runs:?}");
    }
    #[test]
    fn scene_preserves_catalog_structures_and_page_annotations() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("structured.pdf"); fixture(&input);
        let mut source=Document::load(&input).unwrap();
        let pages:Vec<_>=source.get_pages().values().copied().collect();
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
        let scene=PdfScene { pages: pages.iter().enumerate().map(|(index,_)| ScenePage { source_index:Some(index), width:if index==0 {300.} else {612.}, height:if index==0 {200.} else {792.}, rotation:if index==0 {90} else {0}, crop:None, objects:vec![] }).collect() };
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
    fn scene_rejects_nested_page_tree_before_output() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("nested.pdf"); nested_fixture(&input);
        let scene=PdfScene { pages:vec![ScenePage { source_index:Some(0), width:300., height:200., rotation:0, crop:None, objects:vec![] }, ScenePage { source_index:Some(1), width:612., height:792., rotation:0, crop:None, objects:vec![] }] };
        let result=export(&ExportRequest {paths:vec![input.clone()],scene,output_location:OutputLocation::AlongsideInput},input.clone());
        assert!(result.failure.as_ref().is_some_and(|error| error.message.contains("nested")));
        assert!(result.output_paths.is_empty());
        assert!(!input.with_file_name("nested-edited-1.pdf").exists());
    }
    #[test]
    fn scene_rejects_annotation_crop_before_output() {
        let temp=TempDir::new().unwrap(); let input=temp.0.join("annotated.pdf"); fixture(&input);
        let mut source=Document::load(&input).unwrap(); let page=source.get_pages().values().next().copied().unwrap();
        let annotation=source.add_object(dictionary! {"Type"=>"Annot", "Subtype"=>"Text", "Rect"=>array(&[20.,20.,80.,50.])});
        source.get_dictionary_mut(page).unwrap().set("Annots",vec![Object::Reference(annotation)]); source.save(&input).unwrap();
        let scene=PdfScene { pages:vec![ScenePage { source_index:Some(0), width:300., height:200., rotation:0, crop:Some(Rect{x:0.,y:0.,width:100.,height:100.}), objects:vec![] }] };
        let result=export(&ExportRequest {paths:vec![input.clone()],scene,output_location:OutputLocation::AlongsideInput},input.clone());
        assert!(result.failure.as_ref().is_some_and(|error| error.message.contains("annotation")));
        assert!(result.output_paths.is_empty());
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
            ScenePage {source_index:Some(0),width:300.,height:200.,rotation:0,crop:None,objects:vec![]},
            ScenePage {source_index:Some(1),width:612.,height:792.,rotation:0,crop:None,objects:vec![]},
        ] };
        let result=export(&ExportRequest {paths:vec![input.clone()],scene,output_location:OutputLocation::AlongsideInput},input.clone());
        assert!(result.failure.as_ref().is_some_and(|error| error.message.contains("Digitally signed")));
        assert!(result.output_paths.is_empty());
        assert!(!input.with_file_name("signed-edited-1.pdf").exists());
    }
}
