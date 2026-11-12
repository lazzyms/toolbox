//! Reads an OpenDocument presentation (ODP) into the shared slide model, drawing each slide's shapes with the OpenDocument text reader.

use crate::kit::contracts::ToolError;
use crate::kit::pdf::layout::{Block, Float, Frame, Place, Position};
use crate::kit::pdf::odt::{manifest, Converter, Props, Styles};
use crate::kit::pdf::presentation::{slide_side, Deck, Include, Slide, DEFAULT_SLIDE};
use crate::kit::pdf::wordprocessing::{self as shared, css_points, hex_color, Families, Node, Out, Package};

pub const DAMAGED: &str = "This OpenDocument presentation is damaged or incomplete, so no PDF was made.";
const CONTENT: &str = "content.xml";
/// Shapes whose outline is not a rectangle; they print as their bounding box.
const ROUNDED: &[&str] = &["ellipse", "circle", "regular-polygon"];

fn damaged() -> ToolError { ToolError::invalid_input(DAMAGED) }

fn tree(bytes: &[u8]) -> Result<Node, ToolError> { shared::tree(bytes, DAMAGED) }

fn in_body(open: &[Node]) -> bool { open.len() == 3 && open[1].name == "body" && open[2].name == "presentation" }

/// Reads an ODP package. `budget` caps the bytes expanded across every part read; going over returns `exceeded`, and a password-protected package returns `locked`.
/// Hidden slides and speaker notes are only read when `include` asks for them.
pub fn parse(bytes: &[u8], budget: u64, exceeded: ToolError, locked: ToolError, include: Include) -> Result<Deck, ToolError> {
    let mut package = Package::open(bytes, budget, exceeded, DAMAGED)?;
    let media = manifest(&mut package, DAMAGED, locked)?;
    let mut styles = Styles::default();
    if let Some(bytes) = package.part("styles.xml")? { styles.read(&tree(&bytes)?); }
    let content = package.part(CONTENT)?.ok_or_else(damaged)?;
    let head = shared::walk(&content, in_body, DAMAGED, |_| Ok(()))?;
    if head.name != "document-content" || head.child("body").and_then(|body| body.child("presentation")).is_none() { return Err(damaged()); }
    styles.read(&head);
    let embedded = styles.embedded(&mut package)?;

    let fonts = Families { body: styles.font(&styles.drawing_text(&Node::default())), others: Vec::new() };
    let mut converter = Converter::new(package, styles, media, fonts);
    converter.slides = true;
    let mut slides = Vec::new();
    let mut hidden_left_out = 0;
    let mut number = 0;
    shared::walk(&content, in_body, DAMAGED, |page| {
        if page.name != "page" { return Ok(()); }
        number += 1;
        let hidden = converter.styles.drawing_page(page.attr("style-name")).get("visibility").map(String::as_str) == Some("hidden");
        if hidden && !include.hidden {
            hidden_left_out += 1;
            return Ok(());
        }
        slides.push(slide(&mut converter, number, &page, include)?);
        Ok(())
    })?;

    let (title, author) = match converter.package.part("meta.xml")? {
        Some(bytes) => {
            let meta = tree(&bytes)?;
            let field = |name: &str| meta.find(name).map(|node| node.text().trim().to_string()).filter(|text| !text.is_empty());
            (field("title"), field("initial-creator").or_else(|| field("creator")))
        }
        None => (None, None),
    };
    Ok(Deck {
        slides,
        hidden_left_out,
        pictures: converter.pictures,
        families: converter.fonts.others,
        body_family: converter.fonts.body,
        embedded,
        title,
        author,
    })
}

fn slide(converter: &mut Converter, number: usize, page: &Node, include: Include) -> Result<Slide, ToolError> {
    let master_name = page.attr("master-page-name");
    let size = converter.styles.page_layout(master_name).and_then(|layout| {
        let side = |name: &str| layout.attr(name).and_then(css_points).filter(|side| *side > 0.);
        Some((slide_side(side("page-width")?), slide_side(side("page-height")?)))
    }).unwrap_or(DEFAULT_SLIDE);
    let master = converter.styles.master(master_name).cloned();

    let mut approximate = 0;
    let mut out = Out::default();
    let background = background(&converter.styles.drawing_page(page.attr("style-name")))
        .or_else(|| background(&converter.styles.drawing_page(master.as_ref().and_then(|master| master.attr("style-name")))));
    if let Some(fill) = background {
        out.push(Block::Float(Float {
            x: Position { frame: Frame::Page, place: Place::Offset(0.) },
            y: Position { frame: Frame::Page, place: Place::Offset(0.) },
            width: size.0,
            height: size.1,
            fill: Some(fill),
            line: None,
            picture: None,
            blocks: Vec::new(),
            wrap: false,
        }));
    }
    let decorations = master.iter().flat_map(|master| master.nodes()).filter(|node| node.attr("class").is_none());
    for shape in decorations.chain(page.nodes().filter(|node| node.attr("placeholder") != Some("true"))) {
        if matches!(shape.name.as_str(), "notes" | "forms") { continue; }
        let (shape, rough) = placed(shape);
        if rough || ROUNDED.contains(&shape.name.as_str()) || custom_outline(&shape) { approximate += 1; }
        converter.block(&shape, &mut out)?;
    }
    let title = page.nodes().find(|node| node.attr("class") == Some("title"))
        .map(|frame| frame.text().split_whitespace().collect::<Vec<_>>().join(" ")).filter(|title| !title.is_empty());

    let mut notes = Vec::new();
    if include.notes {
        for frame in page.nodes().filter(|node| node.name == "notes").flat_map(|notes| notes.nodes()).filter(|node| node.attr("class") == Some("notes")) {
            if let Some(content) = frame.child("text-box") { notes.extend(converter.drawing_text(frame, content)?); }
        }
    }
    let notes = if notes.iter().any(|block| !matches!(block, Block::Formatted(paragraph) if paragraph.spans.iter().all(|span| span.text.trim().is_empty()))) { notes } else { Vec::new() };
    Ok(Slide { number, size, title, shapes: out.finish(), notes, approximate, omitted: std::mem::take(&mut converter.omitted) })
}

fn background(page: &Props) -> Option<[u8; 3]> {
    (page.get("fill").map(String::as_str) == Some("solid")).then(|| page.get("fill-color").and_then(|colour| hex_color(colour.trim().trim_start_matches('#')))).flatten()
}

/// A shape placed by a transform instead of a position takes its position from the transform's translation; rotated and skewed shapes print as their unrotated box.
fn placed(shape: &Node) -> (Node, bool) {
    let mut shape = shape.clone();
    let Some(transform) = shape.attr("transform").map(str::to_string) else { return (shape, false) };
    let rough = transform.contains("rotate") || transform.contains("skew") || transform.contains("matrix");
    if shape.attr("x").is_none() {
        if let Some(arguments) = transform.split("translate").nth(1).and_then(|rest| rest.trim_start().strip_prefix('(')).and_then(|rest| rest.split(')').next()) {
            let mut lengths = arguments.split([' ', ',']).filter(|part| !part.is_empty()).filter_map(css_points);
            if let Some(x) = lengths.next() {
                let y = lengths.next().unwrap_or(0.);
                shape.attrs.push(("x".into(), format!("{x}pt")));
                shape.attrs.push(("y".into(), format!("{y}pt")));
            }
        }
    }
    (shape, rough)
}

fn custom_outline(shape: &Node) -> bool {
    shape.name == "custom-shape" && shape.child("enhanced-geometry").and_then(|geometry| geometry.attr("type")).is_none_or(|kind| kind != "rectangle")
}

#[cfg(test)]
pub mod fixtures {
    use std::io::{Cursor, Write};

    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    pub use crate::kit::pdf::odt::fixtures::NS;

    pub const PRESENTATION_NS: &str = r#"xmlns:presentation="urn:oasis:names:tc:opendocument:xmlns:presentation:1.0""#;

    /// Builds an ODP package from shared styles, automatic styles and the slides inside `office:presentation`.
    /// Extra parts such as pictures are added as given. The styles part always defines a "Default" master page on a 10 by 7.5 inch page layout unless `styles` replaces it.
    pub fn odp(styles: &str, automatic: &str, body: &str, parts: &[(&str, &[u8])]) -> Vec<u8> {
        let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
        archive.start_file("mimetype", SimpleFileOptions::default().compression_method(CompressionMethod::Stored)).unwrap();
        archive.write_all(b"application/vnd.oasis.opendocument.presentation").unwrap();
        let mut add = |name: &str, data: &[u8]| {
            archive.start_file(name, SimpleFileOptions::default()).unwrap();
            archive.write_all(data).unwrap();
        };
        add("META-INF/manifest.xml", br#"<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.3"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.presentation"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="styles.xml" manifest:media-type="text/xml"/></manifest:manifest>"#);
        let styles = if styles.is_empty() {
            r#"<office:automatic-styles><style:page-layout style:name="PM1"><style:page-layout-properties fo:page-width="10in" fo:page-height="7.5in"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Default" style:page-layout-name="PM1"/></office:master-styles>"#
        } else { styles };
        add("styles.xml", format!(r#"<?xml version="1.0" encoding="UTF-8"?><office:document-styles {NS} {PRESENTATION_NS}>{styles}</office:document-styles>"#).as_bytes());
        add("content.xml", format!(r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content {NS} {PRESENTATION_NS}><office:automatic-styles>{automatic}</office:automatic-styles><office:body><office:presentation>{body}</office:presentation></office:body></office:document-content>"#).as_bytes());
        for (name, data) in parts { add(name, data); }
        archive.finish().unwrap().into_inner()
    }
}
