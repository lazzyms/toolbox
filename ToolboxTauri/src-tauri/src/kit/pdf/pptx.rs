//! Reads a PowerPoint presentation (PPTX) into the shared slide model: each shape becomes a box at its place on the slide.

use std::collections::HashMap;
use std::rc::Rc;

use crate::kit::contracts::ToolError;
use crate::kit::pdf::docx::shape_paint;
use crate::kit::pdf::layout::{plain_text, Align, Block, Float, Frame, Omitted, Paragraph, Picture, Place, Position, Span, Style};
use crate::kit::pdf::presentation::{slide_side, Deck, Include, Slide, DEFAULT_SLIDE};
use crate::kit::pdf::wordprocessing::{self as shared, hex_color, rels, safe_uri, Families, Node, Package, Rel, SHAPE_LINE};

pub const DAMAGED: &str = "This PowerPoint presentation is damaged or incomplete, so no PDF was made.";
const PRESENTATION: &str = "ppt/presentation.xml";
const EMU_PER_POINT: f32 = 12_700.;
const LEVELS: usize = 9;
const DEFAULT_SIZE: f32 = 18.;
const NOTES_SIZE: f32 = 12.;
const LEADING: f32 = 1.2;
const FULL_TURN: i64 = 21_600_000;

type Rels = HashMap<String, Rel>;

fn damaged() -> ToolError { ToolError::invalid_input(DAMAGED) }

fn tree(bytes: &[u8]) -> Result<Node, ToolError> { shared::tree(bytes, DAMAGED) }

fn points(value: Option<&str>) -> Option<f32> { value?.trim().parse::<f64>().ok().filter(|value| value.is_finite()).map(|value| (value / EMU_PER_POINT as f64) as f32) }

fn number(value: Option<&str>) -> Option<f32> { value?.trim().parse::<f32>().ok().filter(|value| value.is_finite()) }

fn flag(value: Option<&str>) -> Option<bool> { value.map(|value| matches!(value, "1" | "true" | "on")) }

/// The relationship an element points at. Attribute names are read without their prefix, so `r:id` and a plain `id` are told apart by which one names a relationship.
fn related<'r>(node: &Node, name: &str, rels: &'r Rels) -> Option<&'r Rel> {
    node.attrs.iter().filter(|(key, _)| key == name).find_map(|(_, value)| rels.get(value)).filter(|rel| !rel.external)
}

fn target<'r>(rels: &'r Rels, kind: &str) -> Option<&'r str> {
    rels.values().find(|rel| !rel.external && rel.kind.ends_with(kind)).map(|rel| rel.target.as_str())
}

/// Reads a PPTX package. `budget` caps the bytes expanded across every part read; going over returns `exceeded`.
/// Hidden slides and speaker notes are only read when `include` asks for them.
pub fn parse(bytes: &[u8], budget: u64, exceeded: ToolError, include: Include) -> Result<Deck, ToolError> {
    let mut package = Package::open(bytes, budget, exceeded, DAMAGED)?;
    let presentation = tree(&package.part(PRESENTATION)?.ok_or_else(damaged)?)?;
    if presentation.name != "presentation" { return Err(damaged()); }
    let presentation_rels = rels(&mut package, PRESENTATION)?;
    let size = presentation.child("sldSz").and_then(|size| Some((points(size.attr("cx"))?, points(size.attr("cy"))?)))
        .filter(|(width, height)| *width > 0. && *height > 0.)
        .map_or(DEFAULT_SLIDE, |(width, height)| (slide_side(width), slide_side(height)));

    let mut reader = Reader {
        package,
        defaults: presentation.child("defaultTextStyle").cloned(),
        layouts: HashMap::new(),
        masters: HashMap::new(),
        fallback: None,
        pictures: Vec::new(),
        cache: HashMap::new(),
        fonts: Families::default(),
        omitted: Omitted::default(),
        approximate: 0,
    };
    let order = presentation.child("sldIdLst").map(|list| {
        list.nodes().filter(|node| node.name == "sldId").filter_map(|node| related(node, "id", &presentation_rels).map(|rel| rel.target.clone())).collect::<Vec<_>>()
    }).unwrap_or_default();

    let mut slides = Vec::new();
    let mut hidden_left_out = 0;
    for (index, path) in order.iter().enumerate() {
        let root = tree(&reader.package.part(path)?.ok_or_else(damaged)?)?;
        if root.name != "sld" { return Err(damaged()); }
        if flag(root.attr("show")) == Some(false) && !include.hidden {
            hidden_left_out += 1;
            continue;
        }
        let slide_rels = rels(&mut reader.package, path)?;
        slides.push(reader.slide(index + 1, size, &root, &slide_rels, include)?);
    }

    let (title, author) = match reader.package.part("docProps/core.xml")? {
        Some(bytes) => {
            let core = tree(&bytes)?;
            let field = |name: &str| core.child(name).map(|node| node.text().trim().to_string()).filter(|text| !text.is_empty());
            (field("title"), field("creator"))
        }
        None => (None, None),
    };
    Ok(Deck {
        slides,
        hidden_left_out,
        pictures: reader.pictures,
        families: reader.fonts.others,
        body_family: reader.fonts.body,
        embedded: Vec::new(),
        title,
        author,
    })
}

/// Theme colours and fonts as a slide master maps them.
#[derive(Debug, Default)]
struct Palette { colours: HashMap<String, [u8; 3]>, map: HashMap<String, String>, minor: Option<String>, major: Option<String> }

impl Palette {
    fn read(theme: Option<&Node>, map: Option<&Node>) -> Self {
        let face = |name: &str| theme?.find(name)?.child("latin")?.attr("typeface").filter(|face| !face.is_empty()).map(str::to_string);
        let mut palette = Self { minor: face("minorFont"), major: face("majorFont"), ..Self::default() };
        if let Some(scheme) = theme.and_then(|theme| theme.find("clrScheme")) {
            let colours = scheme.nodes().filter_map(|slot| Some((slot.name.clone(), palette.colour(slot)?))).collect();
            palette.colours = colours;
        }
        palette.map = match map {
            Some(map) => map.attrs.iter().cloned().collect(),
            None => [("bg1", "lt1"), ("tx1", "dk1"), ("bg2", "lt2"), ("tx2", "dk2")].iter().map(|(from, to)| (from.to_string(), to.to_string())).collect(),
        };
        palette
    }

    /// The colour held by a fill, style reference or colour slot. Tints and shades applied to it are ignored.
    fn colour(&self, holder: &Node) -> Option<[u8; 3]> {
        holder.nodes().find_map(|colour| match colour.name.as_str() {
            "srgbClr" => colour.attr("val").and_then(hex_color),
            "sysClr" => colour.attr("lastClr").and_then(hex_color).or(match colour.attr("val") {
                Some("window") => Some([255, 255, 255]),
                Some(_) => Some([0, 0, 0]),
                None => None,
            }),
            "schemeClr" => {
                let name = colour.attr("val")?;
                let slot = self.map.get(name).map_or(name, String::as_str);
                self.colours.get(slot).copied()
            }
            "prstClr" => match colour.attr("val")? {
                "black" => Some([0, 0, 0]),
                "white" => Some([255, 255, 255]),
                "red" => Some([255, 0, 0]),
                "green" => Some([0, 128, 0]),
                "blue" => Some([0, 0, 255]),
                "yellow" => Some([255, 255, 0]),
                "gray" | "grey" => Some([128, 128, 128]),
                _ => None,
            },
            _ => None,
        })
    }

    fn font(&self, typeface: &str) -> Option<String> {
        match typeface {
            "" => None,
            face if face.starts_with("+mj") => self.major.clone(),
            face if face.starts_with("+mn") => self.minor.clone(),
            face => Some(face.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Bullet { None, Char(String), Number(u64) }

/// Paragraph and run properties at one outline level. Unset values come from the level below in the inheritance chain.
#[derive(Debug, Clone, Default)]
struct Text {
    size: Option<f32>,
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    strike: Option<bool>,
    color: Option<[u8; 3]>,
    font: Option<String>,
    align: Option<Align>,
    margin: Option<f32>,
    before: Option<f32>,
    bullet: Option<Bullet>,
}

impl Text {
    fn run(properties: &Node, palette: &Palette) -> Self {
        Self {
            size: number(properties.attr("sz")).map(|size| size / 100.).filter(|size| *size > 0.),
            bold: flag(properties.attr("b")),
            italic: flag(properties.attr("i")),
            underline: properties.attr("u").map(|value| value != "none"),
            strike: properties.attr("strike").map(|value| value != "noStrike"),
            color: properties.child("solidFill").and_then(|fill| palette.colour(fill)),
            font: properties.child("latin").and_then(|latin| latin.attr("typeface")).and_then(|face| palette.font(face)),
            ..Self::default()
        }
    }

    fn paragraph(properties: &Node, palette: &Palette) -> Self {
        let run = properties.child("defRPr").map(|run| Self::run(run, palette)).unwrap_or_default();
        let bullet = properties.nodes().find_map(|node| match node.name.as_str() {
            "buNone" => Some(Bullet::None),
            "buChar" => Some(Bullet::Char(node.attr("char").unwrap_or("\u{2022}").to_string())),
            "buAutoNum" => Some(Bullet::Number(node.attr("startAt").and_then(|start| start.parse().ok()).unwrap_or(1))),
            _ => None,
        });
        let before = properties.child("spcBef").and_then(|space| {
            space.child("spcPts").and_then(|points| number(points.attr("val"))).map(|value| value / 100.)
                .or_else(|| space.child("spcPct").and_then(|percent| number(percent.attr("val"))).map(|value| -value / 100_000.))
        });
        Self {
            align: properties.attr("algn").map(|align| match align {
                "ctr" => Align::Center,
                "r" => Align::Right,
                _ => Align::Left,
            }),
            margin: points(properties.attr("marL")).map(|margin| margin.max(0.)),
            before,
            bullet,
            ..run
        }
    }

    /// These properties with `top` laid over them.
    fn over(&self, top: &Self) -> Self {
        Self {
            size: top.size.or(self.size),
            bold: top.bold.or(self.bold),
            italic: top.italic.or(self.italic),
            underline: top.underline.or(self.underline),
            strike: top.strike.or(self.strike),
            color: top.color.or(self.color),
            font: top.font.clone().or_else(|| self.font.clone()),
            align: top.align.or(self.align),
            margin: top.margin.or(self.margin),
            before: top.before.or(self.before),
            bullet: top.bullet.clone().or_else(|| self.bullet.clone()),
        }
    }

    /// Space before the paragraph in points; a negative value is a share of the font size.
    fn space_before(&self, size: f32) -> f32 {
        match self.before { Some(before) if before < 0. => -before * size, Some(before) => before, None => 0. }
    }
}

/// The nine outline levels of a list style (`lvl1pPr` to `lvl9pPr`), each read on its own.
fn levels(list: Option<&Node>, palette: &Palette) -> Vec<Text> {
    (1..=LEVELS).map(|level| list.and_then(|list| list.child(&format!("lvl{level}pPr"))).map(|node| Text::paragraph(node, palette)).unwrap_or_default()).collect()
}

fn layered(base: &[Text], top: &[Text]) -> Vec<Text> { base.iter().zip(top).map(|(base, top)| base.over(top)).collect() }

fn sized(size: f32) -> Vec<Text> { vec![Text { size: Some(size), ..Text::default() }; LEVELS] }

/// A slide master: its shape tree, colours and fonts, and the text styles of its title, body and other text.
struct Master { root: Node, rels: Rels, palette: Palette, title: Vec<Text>, body: Vec<Text>, other: Vec<Text>, defaults: Vec<Text> }

struct Layout { root: Node, rels: Rels, master: Rc<Master> }

/// Which placeholder a shape fills: its kind, folded so that a layout's centred title matches the master's title, and its index.
#[derive(Debug, Clone, PartialEq)]
struct Placeholder { kind: &'static str, index: Option<String> }

fn placeholder(shape: &Node) -> Option<Placeholder> {
    let ph = shape.nodes().find(|node| node.name.starts_with("nv"))?.child("nvPr")?.child("ph")?;
    let kind = match ph.attr("type") {
        Some("title" | "ctrTitle") => "title",
        None | Some("body" | "subTitle" | "obj") => "body",
        Some("dt") => "dt",
        Some("ftr") => "ftr",
        Some("sldNum") => "sldNum",
        Some(_) => "other",
    };
    Some(Placeholder { kind, index: ph.attr("idx").map(str::to_string) })
}

/// The placeholder shape in a layout or master shape tree that `wanted` inherits from: the same index first, else the same kind.
fn inherited<'t>(shapes: &'t Node, wanted: &Placeholder) -> Option<&'t Node> {
    let candidates = || shapes.nodes().filter(|node| node.name == "sp").filter_map(|node| Some((node, placeholder(node)?)));
    wanted.index.as_ref().and_then(|index| candidates().find(|(_, found)| found.index.as_ref() == Some(index)).map(|(node, _)| node))
        .or_else(|| candidates().find(|(_, found)| found.kind == wanted.kind).map(|(node, _)| node))
}

fn shape_tree(root: &Node) -> Option<&Node> { root.child("cSld")?.child("spTree") }

/// Maps a group's child coordinates onto the slide, in points.
#[derive(Debug, Clone, Copy)]
struct Transform { dx: f32, dy: f32, sx: f32, sy: f32 }

impl Transform {
    const IDENTITY: Self = Self { dx: 0., dy: 0., sx: 1., sy: 1. };

    fn group(self, xfrm: &Node) -> Self {
        let pair = |name: &str, first: &str, second: &str| xfrm.child(name).and_then(|node| Some((points(node.attr(first))?, points(node.attr(second))?)));
        let (Some(offset), Some(extent)) = (pair("off", "x", "y"), pair("ext", "cx", "cy")) else { return self };
        let child_offset = pair("chOff", "x", "y").unwrap_or(offset);
        let child_extent = pair("chExt", "cx", "cy").unwrap_or(extent);
        let scale = |extent: f32, child: f32| if child > 0. { extent / child } else { 1. };
        let (sx, sy) = (scale(extent.0, child_extent.0), scale(extent.1, child_extent.1));
        Self {
            dx: self.dx + self.sx * (offset.0 - child_offset.0 * sx),
            dy: self.dy + self.sy * (offset.1 - child_offset.1 * sy),
            sx: self.sx * sx,
            sy: self.sy * sy,
        }
    }

    fn rect(self, xfrm: &Node) -> Option<(f32, f32, f32, f32)> {
        let offset = xfrm.child("off")?;
        let extent = xfrm.child("ext")?;
        let (x, y) = (points(offset.attr("x"))?, points(offset.attr("y"))?);
        let (width, height) = (points(extent.attr("cx"))?, points(extent.attr("cy"))?);
        Some((self.dx + x * self.sx, self.dy + y * self.sy, (width * self.sx).max(0.), (height * self.sy).max(0.)))
    }
}

fn rotated(xfrm: &Node) -> bool { xfrm.attr("rot").and_then(|rot| rot.trim().parse::<i64>().ok()).is_some_and(|rot| rot % FULL_TURN != 0) }

/// Whether a shape's outline is something other than a rectangle.
fn shaped(properties: Option<&Node>) -> bool {
    properties.is_some_and(|properties| properties.child("custGeom").is_some() || properties.child("prstGeom").and_then(|geometry| geometry.attr("prst")).is_some_and(|preset| preset != "rect"))
}

fn float(rect: (f32, f32, f32, f32), fill: Option<[u8; 3]>, line: Option<[u8; 3]>, picture: Option<usize>, blocks: Vec<Block>) -> Block {
    let (x, y, width, height) = rect;
    Block::Float(Float {
        x: Position { frame: Frame::Page, place: Place::Offset(x) },
        y: Position { frame: Frame::Page, place: Place::Offset(y) },
        width,
        height,
        fill,
        line,
        picture,
        blocks,
        wrap: false,
    })
}

fn has_text(blocks: &[Block]) -> bool {
    blocks.iter().any(|block| match block {
        Block::Formatted(paragraph) => !plain_text(&paragraph.spans).trim().is_empty(),
        _ => true,
    })
}

/// Where the shapes being read come from: the part's relationships, its master, and the layout and master shape trees its placeholders inherit from.
struct Scope<'s> { rels: &'s Rels, master: &'s Master, inherits: Vec<&'s Node>, placeholders: bool }

#[derive(Default)]
struct Built { blocks: Vec<Block>, title: Option<String> }

struct Reader<'a> {
    package: Package<'a>,
    defaults: Option<Node>,
    layouts: HashMap<String, Rc<Layout>>,
    masters: HashMap<String, Rc<Master>>,
    fallback: Option<Rc<Master>>,
    pictures: Vec<Picture>,
    cache: HashMap<String, usize>,
    fonts: Families,
    omitted: Omitted,
    approximate: usize,
}

impl Reader<'_> {
    fn master(&mut self, path: Option<&str>) -> Result<Rc<Master>, ToolError> {
        if let Some(master) = path.and_then(|path| self.masters.get(path)) { return Ok(master.clone()); }
        let root = match path { Some(path) => self.package.part(path)?.map(|bytes| tree(&bytes)).transpose()?, None => None };
        let Some((path, root)) = path.zip(root) else {
            if let Some(fallback) = &self.fallback { return Ok(fallback.clone()); }
            let fallback = Rc::new(self.build_master(Node { name: "sldMaster".into(), ..Node::default() }, Rels::new(), None));
            self.fallback = Some(fallback.clone());
            return Ok(fallback);
        };
        if root.name != "sldMaster" { return Err(damaged()); }
        let master_rels = rels(&mut self.package, path)?;
        let theme = match target(&master_rels, "/theme") {
            Some(theme) => self.package.part(theme)?.map(|bytes| tree(&bytes)).transpose()?,
            None => None,
        };
        let master = Rc::new(self.build_master(root, master_rels, theme.as_ref()));
        self.masters.insert(path.to_string(), master.clone());
        Ok(master)
    }

    fn build_master(&mut self, root: Node, rels: Rels, theme: Option<&Node>) -> Master {
        let palette = Palette::read(theme, root.child("clrMap"));
        if self.fonts.body.is_none() { self.fonts.body = palette.minor.clone(); }
        let styles = root.child("txStyles");
        let style = |name: &str| layered(&sized(DEFAULT_SIZE), &levels(styles.and_then(|styles| styles.child(name)), &palette));
        let (title, body, other) = (style("titleStyle"), style("bodyStyle"), style("otherStyle"));
        let defaults = layered(&sized(DEFAULT_SIZE), &levels(self.defaults.as_ref(), &palette));
        Master { root, rels, palette, title, body, other, defaults }
    }

    fn layout(&mut self, path: Option<&str>) -> Result<Rc<Layout>, ToolError> {
        if let Some(layout) = path.and_then(|path| self.layouts.get(path)) { return Ok(layout.clone()); }
        let root = match path { Some(path) => self.package.part(path)?.map(|bytes| tree(&bytes)).transpose()?, None => None };
        let Some((path, root)) = path.zip(root) else {
            let master = self.master(None)?;
            return Ok(Rc::new(Layout { root: Node { name: "sldLayout".into(), ..Node::default() }, rels: Rels::new(), master }));
        };
        if root.name != "sldLayout" { return Err(damaged()); }
        let layout_rels = rels(&mut self.package, path)?;
        let master_path = target(&layout_rels, "/slideMaster").map(str::to_string);
        let master = self.master(master_path.as_deref())?;
        let layout = Rc::new(Layout { root, rels: layout_rels, master });
        self.layouts.insert(path.to_string(), layout.clone());
        Ok(layout)
    }

    fn slide(&mut self, number: usize, size: (f32, f32), root: &Node, slide_rels: &Rels, include: Include) -> Result<Slide, ToolError> {
        let layout_path = target(slide_rels, "/slideLayout").map(str::to_string);
        let layout = self.layout(layout_path.as_deref())?;
        let master = layout.master.clone();
        let mut built = Built::default();

        let sources = [(root, slide_rels), (&layout.root, &layout.rels), (&master.root, &master.rels)];
        if let Some(background) = self.background(&sources, &master.palette, size)? { built.blocks.push(background); }
        let shown = |root: &Node| flag(root.attr("showMasterSp")) != Some(false);
        if shown(root) && shown(&layout.root) {
            if let Some(shapes) = shape_tree(&master.root) {
                let scope = Scope { rels: &master.rels, master: &master, inherits: Vec::new(), placeholders: false };
                self.shapes(shapes, &scope, Transform::IDENTITY, &mut built)?;
            }
        }
        if shown(root) {
            if let Some(shapes) = shape_tree(&layout.root) {
                let scope = Scope { rels: &layout.rels, master: &master, inherits: shape_tree(&master.root).into_iter().collect(), placeholders: false };
                self.shapes(shapes, &scope, Transform::IDENTITY, &mut built)?;
            }
        }
        if let Some(shapes) = shape_tree(root) {
            let scope = Scope { rels: slide_rels, master: &master, inherits: shape_tree(&layout.root).into_iter().chain(shape_tree(&master.root)).collect(), placeholders: true };
            self.shapes(shapes, &scope, Transform::IDENTITY, &mut built)?;
        }

        let notes = match target(slide_rels, "/notesSlide").filter(|_| include.notes).map(str::to_string) {
            Some(path) => self.notes(&path, &master)?,
            None => Vec::new(),
        };
        Ok(Slide {
            number,
            size,
            title: built.title,
            shapes: built.blocks,
            notes,
            approximate: std::mem::take(&mut self.approximate),
            omitted: std::mem::take(&mut self.omitted),
        })
    }

    /// The first background stated by the slide, its layout or its master, drawn across the whole slide.
    fn background(&mut self, sources: &[(&Node, &Rels)], palette: &Palette, size: (f32, f32)) -> Result<Option<Block>, ToolError> {
        for (root, rels) in sources {
            let Some(background) = root.child("cSld").and_then(|slide| slide.child("bg")) else { continue };
            let (mut fill, mut picture) = (None, None);
            if let Some(properties) = background.child("bgPr") {
                if let Some(solid) = properties.child("solidFill") { fill = palette.colour(solid); }
                if let Some(gradient) = properties.child("gradFill") { fill = gradient.find("gs").and_then(|stop| palette.colour(stop)); }
                if let Some(pattern) = properties.child("pattFill") { fill = pattern.child("fgClr").and_then(|colour| palette.colour(colour)); }
                if let Some(blip) = properties.child("blipFill").and_then(|blip| blip.child("blip")) {
                    picture = self.picture(blip, rels, Some(size))?;
                    if picture.is_none() { self.omitted.pictures += 1; }
                }
            } else if let Some(reference) = background.child("bgRef") {
                fill = palette.colour(reference);
            }
            if fill.is_none() && picture.is_none() { return Ok(None); }
            return Ok(Some(float((0., 0., size.0, size.1), fill, None, picture, Vec::new())));
        }
        Ok(None)
    }

    fn shapes(&mut self, shapes: &Node, scope: &Scope, transform: Transform, built: &mut Built) -> Result<(), ToolError> {
        for node in shapes.nodes() {
            let is_placeholder = placeholder(node).is_some();
            if is_placeholder && !scope.placeholders { continue; }
            match node.name.as_str() {
                "sp" => self.shape(node, scope, transform, built)?,
                "pic" => self.pic(node, scope, transform, built)?,
                "graphicFrame" => self.frame(node, scope, transform, built)?,
                "cxnSp" => self.connector(node, scope, transform, built),
                "grpSp" => {
                    let xfrm = node.child("grpSpPr").and_then(|properties| properties.child("xfrm"));
                    if xfrm.is_some_and(rotated) { self.approximate += 1; }
                    self.shapes(node, scope, xfrm.map_or(transform, |xfrm| transform.group(xfrm)), built)?;
                }
                "AlternateContent" => {
                    // A chart's fallback is drawn only when it is the chart's stored image; a fallback note such as "this chart isn't available" is not printed.
                    let chart = node.nodes().filter(|branch| branch.name == "Choice").any(|choice| choice.find("graphicData").is_some_and(|data| matches!(graphic_kind(data), "chart" | "chartex")));
                    if chart && !node.child("Fallback").is_some_and(|fallback| fallback.find("blip").is_some()) {
                        self.omitted.charts += 1;
                        continue;
                    }
                    let branch = node.child("Fallback").or_else(|| node.child("Choice"));
                    if let Some(branch) = branch { self.shapes(branch, scope, transform, built)?; }
                }
                "contentPart" => self.omitted.objects += 1,
                _ => {}
            }
        }
        Ok(())
    }

    fn shape(&mut self, shape: &Node, scope: &Scope, transform: Transform, built: &mut Built) -> Result<(), ToolError> {
        let wanted = placeholder(shape);
        let parents: Vec<&Node> = wanted.as_ref().map(|wanted| scope.inherits.iter().filter_map(|tree| inherited(tree, wanted)).collect()).unwrap_or_default();
        let properties = shape.child("spPr");
        let xfrm = properties.and_then(|properties| properties.child("xfrm"))
            .or_else(|| parents.iter().find_map(|parent| parent.child("spPr")?.child("xfrm")));
        let palette = &scope.master.palette;

        let base = match wanted.as_ref().map(|wanted| wanted.kind) {
            Some("title") => &scope.master.title,
            Some("body") => &scope.master.body,
            Some(_) => &scope.master.other,
            None => &scope.master.defaults,
        };
        let mut text = base.clone();
        for parent in parents.iter().rev() { text = layered(&text, &levels(parent.child("txBody").and_then(|body| body.child("lstStyle")), palette)); }
        let body = shape.child("txBody");
        text = layered(&text, &levels(body.and_then(|body| body.child("lstStyle")), palette));
        let blocks = match body { Some(body) => self.text_body(body, &text, scope.rels, palette), None => Vec::new() };
        let blocks = if has_text(&blocks) { blocks } else { Vec::new() };

        let (fill, line) = shape_paint(shape, |holder| palette.colour(holder));
        let Some(rect) = xfrm.and_then(|xfrm| transform.rect(xfrm)) else {
            if !blocks.is_empty() || fill.is_some() || line.is_some() { self.omitted.objects += 1; }
            return Ok(());
        };
        let picture = match properties.and_then(|properties| properties.child("blipFill")).and_then(|fill| fill.child("blip")) {
            Some(blip) => {
                let picture = self.picture(blip, scope.rels, Some((rect.2, rect.3)))?;
                if picture.is_none() { self.omitted.pictures += 1; }
                picture
            }
            None => None,
        };
        if wanted.as_ref().is_some_and(|wanted| wanted.kind == "title") && built.title.is_none() {
            built.title = Some(blocks.iter().filter_map(|block| match block { Block::Formatted(paragraph) => Some(plain_text(&paragraph.spans)), _ => None })
                .collect::<Vec<_>>().join(" ").split_whitespace().collect::<Vec<_>>().join(" ")).filter(|title| !title.is_empty());
        }
        if blocks.is_empty() && fill.is_none() && line.is_none() && picture.is_none() { return Ok(()); }
        if xfrm.is_some_and(rotated) || (shaped(properties) && (fill.is_some() || line.is_some() || picture.is_some())) { self.approximate += 1; }
        built.blocks.push(float(rect, fill, line, picture, blocks));
        Ok(())
    }

    fn pic(&mut self, node: &Node, scope: &Scope, transform: Transform, built: &mut Built) -> Result<(), ToolError> {
        let properties = node.child("spPr");
        let xfrm = properties.and_then(|properties| properties.child("xfrm")).or_else(|| {
            let wanted = placeholder(node)?;
            scope.inherits.iter().filter_map(|tree| inherited(tree, &wanted)).find_map(|parent| parent.child("spPr")?.child("xfrm"))
        });
        let Some(rect) = xfrm.and_then(|xfrm| transform.rect(xfrm)) else {
            self.omitted.pictures += 1;
            return Ok(());
        };
        let Some(picture) = (match node.child("blipFill").and_then(|fill| fill.child("blip")) { Some(blip) => self.picture(blip, scope.rels, Some((rect.2, rect.3)))?, None => None }) else {
            self.omitted.pictures += 1;
            return Ok(());
        };
        if xfrm.is_some_and(rotated) { self.approximate += 1; }
        let (_, line) = shape_paint(node, |holder| scope.master.palette.colour(holder));
        built.blocks.push(float(rect, None, line, Some(picture), Vec::new()));
        Ok(())
    }

    fn frame(&mut self, node: &Node, scope: &Scope, transform: Transform, built: &mut Built) -> Result<(), ToolError> {
        let Some(data) = node.child("graphic").and_then(|graphic| graphic.child("graphicData")) else {
            self.omitted.objects += 1;
            return Ok(());
        };
        let rect = node.child("xfrm").and_then(|xfrm| transform.rect(xfrm));
        match graphic_kind(data) {
            "chart" | "chartex" => self.omitted.charts += 1,
            "diagram" => self.omitted.diagrams += 1,
            "table" => {
                let (Some(rect), Some(table)) = (rect, data.child("tbl")) else {
                    self.omitted.objects += 1;
                    return Ok(());
                };
                let block = self.table(table, scope);
                built.blocks.push(float(rect, None, None, None, vec![block]));
            }
            _ => {
                let picture = match (rect, data.find("blip")) {
                    (Some(rect), Some(blip)) => self.picture(blip, scope.rels, Some((rect.2, rect.3)))?,
                    _ => None,
                };
                match rect.zip(picture) {
                    Some((rect, picture)) => built.blocks.push(float(rect, None, None, Some(picture), Vec::new())),
                    None => self.omitted.objects += 1,
                }
            }
        }
        Ok(())
    }

    fn table(&mut self, table: &Node, scope: &Scope) -> Block {
        let head_row = table.child("tblPr").and_then(|properties| flag(properties.attr("firstRow"))).unwrap_or(false);
        let text = scope.master.defaults.clone();
        let mut rows = Vec::new();
        for row in table.nodes().filter(|node| node.name == "tr") {
            let cells = row.nodes().filter(|node| node.name == "tc").map(|cell| {
                let merged = flag(cell.attr("hMerge")) == Some(true) || flag(cell.attr("vMerge")) == Some(true);
                let Some(body) = cell.child("txBody").filter(|_| !merged) else { return Vec::new() };
                let mut spans = Vec::new();
                for block in self.text_body(body, &text, scope.rels, &scope.master.palette) {
                    let Block::Formatted(paragraph) = block else { continue };
                    if paragraph.spans.is_empty() { continue; }
                    if !spans.is_empty() { spans.push(Span { text: "\n".into(), style: Style::default(), link: None }); }
                    spans.extend(paragraph.spans);
                }
                spans
            }).collect::<Vec<_>>();
            rows.push(cells);
        }
        let head = if head_row && !rows.is_empty() { rows.remove(0) } else { Vec::new() };
        Block::Table { aligns: Vec::new(), head, rows }
    }

    fn connector(&mut self, node: &Node, scope: &Scope, transform: Transform, built: &mut Built) {
        let Some(rect) = node.child("spPr").and_then(|properties| properties.child("xfrm")).and_then(|xfrm| transform.rect(xfrm)) else { return };
        let (_, line) = shape_paint(node, |holder| scope.master.palette.colour(holder));
        let Some(line) = line.or_else(|| node.child("style").is_none().then_some(SHAPE_LINE).filter(|_| node.child("spPr").and_then(|properties| properties.child("ln")).is_some())) else { return };
        if rect.2 > 1. && rect.3 > 1. { self.approximate += 1; }
        built.blocks.push(float(rect, None, Some(line), None, Vec::new()));
    }

    fn picture(&mut self, blip: &Node, rels: &Rels, size: Option<(f32, f32)>) -> Result<Option<usize>, ToolError> {
        let Some(rel) = related(blip, "embed", rels) else { return Ok(None) };
        if let Some(index) = self.cache.get(&rel.target) { return Ok(Some(*index)); }
        let Some(picture) = self.package.part(&rel.target)?.and_then(|bytes| Picture::from_bytes(&bytes, size).ok()) else { return Ok(None) };
        self.pictures.push(picture);
        self.cache.insert(rel.target.clone(), self.pictures.len() - 1);
        Ok(Some(self.pictures.len() - 1))
    }

    fn notes(&mut self, path: &str, master: &Master) -> Result<Vec<Block>, ToolError> {
        let Some(bytes) = self.package.part(path)? else { return Ok(Vec::new()) };
        let root = tree(&bytes)?;
        if root.name != "notes" { return Err(damaged()); }
        let notes_rels = rels(&mut self.package, path)?;
        let text = sized(NOTES_SIZE);
        let mut blocks = Vec::new();
        for shape in shape_tree(&root).into_iter().flat_map(|shapes| shapes.nodes()).filter(|node| node.name == "sp") {
            if placeholder(shape).is_none_or(|wanted| wanted.kind != "body") { continue; }
            let Some(body) = shape.child("txBody") else { continue };
            blocks.extend(self.text_body(body, &text, &notes_rels, &master.palette));
        }
        Ok(if has_text(&blocks) { blocks } else { Vec::new() })
    }

    fn style(&mut self, text: &Text, scale: f32) -> Style {
        let size = text.size.unwrap_or(DEFAULT_SIZE) * scale;
        Style {
            bold: text.bold.unwrap_or(false),
            italic: text.italic.unwrap_or(false),
            code: false,
            strike: text.strike.unwrap_or(false),
            underline: text.underline.unwrap_or(false),
            family: self.fonts.index(text.font.as_deref()),
            half_points: (size * 2.).round().clamp(2., 3_276.) as u16,
            color: text.color,
        }
    }

    /// A text body's paragraphs, with each paragraph's level, bullet and run styles resolved against `levels`.
    fn text_body(&mut self, body: &Node, levels: &[Text], rels: &Rels, palette: &Palette) -> Vec<Block> {
        let scale = body.child("bodyPr").and_then(|properties| properties.child("normAutofit")).and_then(|fit| number(fit.attr("fontScale")))
            .map_or(1., |scale| (scale / 100_000.).clamp(0.1, 1.));
        let mut counters = [0u64; LEVELS];
        let mut blocks = Vec::new();
        for paragraph in body.nodes().filter(|node| node.name == "p") {
            let properties = paragraph.child("pPr");
            let level = properties.and_then(|properties| properties.attr("lvl")).and_then(|level| level.parse::<usize>().ok()).unwrap_or(0).min(LEVELS - 1);
            let text = levels[level].over(&properties.map(|properties| Text::paragraph(properties, palette)).unwrap_or_default());
            let mut spans: Vec<Span> = Vec::new();
            for child in paragraph.nodes() {
                match child.name.as_str() {
                    "r" | "fld" => {
                        let run_properties = child.child("rPr");
                        let run = text.over(&run_properties.map(|run| Text::run(run, palette)).unwrap_or_default());
                        let link = run_properties.and_then(|run| run.child("hlinkClick")).and_then(|click| {
                            click.attrs.iter().filter(|(key, _)| key == "id").find_map(|(_, id)| rels.get(id)).filter(|rel| rel.external).and_then(|rel| safe_uri(&rel.target))
                        });
                        let value = child.child("t").map(Node::text).unwrap_or_default();
                        let style = self.style(&run, scale);
                        push(&mut spans, &value, style, link);
                    }
                    "br" => {
                        let style = self.style(&text, scale);
                        push(&mut spans, "\n", style, None);
                    }
                    _ => {}
                }
            }
            let end = text.over(&paragraph.child("endParaRPr").map(|run| Text::run(run, palette)).unwrap_or_default());
            let filled = !plain_text(&spans).trim().is_empty();
            for deeper in counters.iter_mut().skip(level + 1) { *deeper = 0; }
            let prefix = match (&text.bullet, filled) {
                (Some(Bullet::Char(character)), true) => {
                    counters[level] = 0;
                    Some(format!("{character} "))
                }
                (Some(Bullet::Number(start)), true) => {
                    counters[level] = if counters[level] == 0 { *start } else { counters[level] + 1 };
                    Some(format!("{}. ", counters[level]))
                }
                (_, true) => {
                    counters[level] = 0;
                    None
                }
                _ => None,
            };
            if let Some(prefix) = prefix {
                let style = match spans.first() { Some(span) => span.style, None => self.style(&text, scale) };
                spans.insert(0, Span { text: prefix, style, link: None });
            }
            let size = spans.iter().map(|span| span.style.half_points as f32 / 2.).fold(0., f32::max);
            let size = if size > 0. { size } else { end.size.unwrap_or(DEFAULT_SIZE) * scale };
            blocks.push(Block::Formatted(Paragraph {
                spans,
                align: text.align.unwrap_or(Align::Left),
                indent: text.margin.unwrap_or(0.),
                right_indent: 0.,
                before: text.space_before(size),
                after: 0.,
                size,
                leading: LEADING,
            }));
        }
        blocks
    }
}

fn push(spans: &mut Vec<Span>, value: &str, style: Style, link: Option<String>) {
    if value.is_empty() { return; }
    match spans.last_mut() {
        Some(span) if span.style == style && span.link == link => span.text.push_str(value),
        _ => spans.push(Span { text: value.to_string(), style, link }),
    }
}

fn graphic_kind(data: &Node) -> &str { data.attr("uri").and_then(|uri| uri.rsplit('/').next()).unwrap_or_default() }

#[cfg(test)]
pub mod fixtures {
    use std::io::{Cursor, Write};

    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    pub const P: &str = r#"xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006""#;
    const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

    /// One slide of a fixture deck: the shapes inside its shape tree, whether it is hidden, its speaker notes and extra relationships `(id, type suffix, target)`.
    #[derive(Default)]
    pub struct FixtureSlide<'a> { pub shapes: &'a str, pub hidden: bool, pub notes: Option<&'a str>, pub rels: &'a [(&'a str, &'a str, &'a str)] }

    fn rels(items: &[(String, String, String)]) -> Vec<u8> {
        let items = items.iter().map(|(id, kind, target)| format!(r#"<Relationship Id="{id}" Type="{REL}/{kind}" Target="{target}"/>"#)).collect::<String>();
        format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{items}</Relationships>"#).into_bytes()
    }

    /// A text shape at `(x, y, width, height)` in points, optionally a placeholder of `kind`.
    pub fn text_shape(kind: Option<&str>, rect: (u32, u32, u32, u32), text: &str, extra: &str) -> String {
        let placeholder = kind.map(|kind| format!(r#"<p:nvPr><p:ph type="{kind}"/></p:nvPr>"#)).unwrap_or_else(|| "<p:nvPr/>".into());
        let (x, y, width, height) = (rect.0 * 12_700, rect.1 * 12_700, rect.2 * 12_700, rect.3 * 12_700);
        format!(r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="Shape"/><p:cNvSpPr/>{placeholder}</p:nvSpPr><p:spPr><a:xfrm{extra}><a:off x="{x}" y="{y}"/><a:ext cx="{width}" cy="{height}"/></a:xfrm></p:spPr><p:txBody><a:bodyPr/><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp>"#)
    }

    /// Builds a PPTX package with a slide size in points, one master and layout, and extra parts such as media.
    pub fn pptx(size: (u32, u32), slides: &[FixtureSlide], parts: &[(&str, &[u8])]) -> Vec<u8> {
        let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
        let mut add = |name: &str, data: &[u8]| {
            archive.start_file(name, SimpleFileOptions::default()).unwrap();
            archive.write_all(data).unwrap();
        };
        add("[Content_Types].xml", br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/></Types>"#);
        add("_rels/.rels", &rels(&[("rId1".into(), "officeDocument".into(), "ppt/presentation.xml".into()), ("rId2".into(), "metadata/core-properties".into(), "docProps/core.xml".into())]));
        add("docProps/core.xml", br#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Quarterly review</dc:title><dc:creator>Toolbox tests</dc:creator></cp:coreProperties>"#);
        let ids = (0..slides.len()).map(|index| format!(r#"<p:sldId id="{}" r:id="rId{}"/>"#, 256 + index, index + 2)).collect::<String>();
        add("ppt/presentation.xml", format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:presentation {P}><p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst><p:sldIdLst>{ids}</p:sldIdLst><p:sldSz cx="{}" cy="{}"/></p:presentation>"#, size.0 * 12_700, size.1 * 12_700).as_bytes());
        let mut deck = vec![("rId1".to_string(), "slideMaster".to_string(), "slideMasters/slideMaster1.xml".to_string())];
        deck.extend((0..slides.len()).map(|index| (format!("rId{}", index + 2), "slide".to_string(), format!("slides/slide{}.xml", index + 1))));
        add("ppt/_rels/presentation.xml.rels", &rels(&deck));
        add("ppt/theme/theme1.xml", format!(r#"<a:theme {P} name="Office"><a:themeElements><a:clrScheme name="Office"><a:dk1><a:srgbClr val="000000"/></a:dk1><a:lt1><a:srgbClr val="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="1F2937"/></a:dk2><a:lt2><a:srgbClr val="EEEEEE"/></a:lt2><a:accent1><a:srgbClr val="2563EB"/></a:accent1></a:clrScheme><a:fontScheme name="Office"><a:majorFont><a:latin typeface="Arial"/></a:majorFont><a:minorFont><a:latin typeface="Arial"/></a:minorFont></a:fontScheme></a:themeElements></a:theme>"#).as_bytes());
        let title = text_shape(Some("title"), (36, 24, 600, 60), "", "");
        add("ppt/slideMasters/slideMaster1.xml", format!(r#"<p:sldMaster {P}><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>{title}</p:spTree></p:cSld><p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1"/><p:txStyles><p:titleStyle><a:lvl1pPr><a:defRPr sz="3200" b="1"/></a:lvl1pPr></p:titleStyle><p:bodyStyle><a:lvl1pPr><a:defRPr sz="2000"/></a:lvl1pPr></p:bodyStyle></p:txStyles></p:sldMaster>"#).as_bytes());
        add("ppt/slideMasters/_rels/slideMaster1.xml.rels", &rels(&[("rId1".into(), "theme".into(), "../theme/theme1.xml".into()), ("rId2".into(), "slideLayout".into(), "../slideLayouts/slideLayout1.xml".into())]));
        add("ppt/slideLayouts/slideLayout1.xml", format!(r#"<p:sldLayout {P}><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:spTree></p:cSld></p:sldLayout>"#).as_bytes());
        add("ppt/slideLayouts/_rels/slideLayout1.xml.rels", &rels(&[("rId1".into(), "slideMaster".into(), "../slideMasters/slideMaster1.xml".into())]));
        for (index, slide) in slides.iter().enumerate() {
            let number = index + 1;
            let show = if slide.hidden { r#" show="0""# } else { "" };
            add(&format!("ppt/slides/slide{number}.xml"), format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:sld {P}{show}><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>{}</p:spTree></p:cSld></p:sld>"#, slide.shapes).as_bytes());
            let mut slide_rels = vec![("rIdLayout".to_string(), "slideLayout".to_string(), "../slideLayouts/slideLayout1.xml".to_string())];
            slide_rels.extend(slide.rels.iter().map(|(id, kind, target)| (id.to_string(), kind.to_string(), target.to_string())));
            if let Some(notes) = slide.notes {
                slide_rels.push(("rIdNotes".into(), "notesSlide".into(), format!("../notesSlides/notesSlide{number}.xml")));
                let body = text_shape(Some("body"), (36, 400, 600, 100), notes, "");
                let image = text_shape(Some("sldImg"), (36, 36, 600, 300), "", "");
                add(&format!("ppt/notesSlides/notesSlide{number}.xml"), format!(r#"<p:notes {P}><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>{image}{body}</p:spTree></p:cSld></p:notes>"#).as_bytes());
            }
            add(&format!("ppt/slides/_rels/slide{number}.xml.rels"), &rels(&slide_rels));
        }
        for (name, data) in parts { add(name, data); }
        archive.finish().unwrap().into_inner()
    }
}
