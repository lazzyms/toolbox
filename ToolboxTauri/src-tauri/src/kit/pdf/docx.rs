use std::collections::HashMap;

use crate::kit::contracts::ToolError;
use crate::kit::pdf::font_resolver::{EmbeddedFont, FontStyle};
use crate::kit::pdf::layout::{plain_text, Align, Block, Float, Frame, Omitted, PageSetup, Paragraph, Picture, Place, Position, Span, Style, PAGE_COUNT, PAGE_NUMBER};
pub use crate::kit::pdf::wordprocessing::Parsed;
use crate::kit::pdf::wordprocessing::{self as shared, css_points, DEFAULT_FLOAT_SIDE, SHAPE_FILL, SHAPE_LINE, hex_color, safe_uri, Collected, Families, Item, Node, Out, Package};

pub const DAMAGED: &str = "This Word document is damaged or incomplete, so no PDF was made.";
const DOCUMENT: &str = "word/document.xml";
const MAX_STYLE_CHAIN: usize = 16;
const DEFAULT_HALF_POINTS: u16 = 20;
const EMU_PER_POINT: f32 = 12_700.;

fn damaged() -> ToolError { ToolError::invalid_input(DAMAGED) }

fn tree(bytes: &[u8]) -> Result<Node, ToolError> { shared::tree(bytes, DAMAGED) }

/// Reads a DOCX package. `budget` caps the bytes expanded across every part read; going over returns `exceeded`.
pub fn parse(bytes: &[u8], budget: u64, exceeded: ToolError) -> Result<Parsed, ToolError> {
    let mut package = Package::open(bytes, budget, exceeded, DAMAGED)?;
    let rels = rels(&mut package, DOCUMENT)?;
    let related = |kind: &str| rels.values().find(|rel| !rel.external && rel.kind.ends_with(kind)).map(|rel| rel.target.clone());

    let theme = match related("/theme") {
        Some(path) => match package.part(&path)? {
            Some(bytes) => ThemeFonts::read(&tree(&bytes)?),
            None => ThemeFonts::default(),
        },
        None => ThemeFonts::default(),
    };
    let styles = match related("/styles") {
        Some(path) => match package.part(&path)? { Some(bytes) => Styles::read(&tree(&bytes)?, &theme), None => Styles::default() },
        None => Styles::default(),
    };
    let numbering = match related("/numbering") {
        Some(path) => match package.part(&path)? { Some(bytes) => Numbering::read(&tree(&bytes)?), None => Numbering::default() },
        None => Numbering::default(),
    };
    let embedded = match related("/fontTable") {
        Some(path) => embedded_fonts(&mut package, &path)?,
        None => Vec::new(),
    };

    let (_, base) = styles.paragraph(None);
    let mut converter = Converter {
        fonts: Families { body: base.font.clone(), others: Vec::new() },
        body_half_points: base.half_points.unwrap_or(DEFAULT_HALF_POINTS),
        package,
        theme,
        styles,
        numbering,
        counters: HashMap::new(),
        pictures: Vec::new(),
        omitted: Omitted::default(),
        headings: 0,
        fields: Vec::new(),
        section: None,
        carry: Vec::new(),
    };

    let document = converter.package.part(DOCUMENT)?.ok_or_else(damaged)?;
    let body = Part { rels, band: false };
    let mut out = Out::default();
    let root = shared::walk(&document, |open| open.len() == 2, DAMAGED, |node| converter.block(&node, &body, &mut out))?;
    if root.name != "document" { return Err(damaged()); }
    let blocks = out.finish();

    let section = converter.section.take().unwrap_or_default();
    let header = converter.band(&section, "headerReference", &body)?;
    let footer = converter.band(&section, "footerReference", &body)?;
    let (title, author) = match converter.package.part("docProps/core.xml")? {
        Some(bytes) => {
            let core = tree(&bytes)?;
            let field = |name: &str| core.child(name).map(|node| node.text().trim().to_string()).filter(|text| !text.is_empty());
            (field("title"), field("creator"))
        }
        None => (None, None),
    };

    let twips = |node: Option<&Node>, name: &str| node.and_then(|node| node.attr(name)).and_then(|value| value.parse::<f32>().ok()).map(|value| value / 20.);
    let size = section.child("pgSz");
    let margins = section.child("pgMar");
    let page = PageSetup {
        width: twips(size, "w"),
        height: twips(size, "h"),
        left: twips(margins, "left"),
        right: twips(margins, "right"),
        top: twips(margins, "top").map(f32::abs),
        bottom: twips(margins, "bottom").map(f32::abs),
    };

    Ok(Parsed {
        blocks,
        header,
        footer,
        header_distance: twips(margins, "header"),
        footer_distance: twips(margins, "footer"),
        pictures: converter.pictures,
        families: converter.fonts.others,
        body_family: converter.fonts.body,
        body_size: converter.body_half_points as f32 / 2.,
        embedded,
        title,
        author,
        page,
        omitted: converter.omitted,
    })
}

#[derive(Debug, Clone)]
struct Rel { target: String, kind: String, external: bool }

fn rels(package: &mut Package, part: &str) -> Result<HashMap<String, Rel>, ToolError> {
    let (folder, file) = part.rsplit_once('/').unwrap_or(("", part));
    let path = if folder.is_empty() { format!("_rels/{file}.rels") } else { format!("{folder}/_rels/{file}.rels") };
    let Some(bytes) = package.part(&path)? else { return Ok(HashMap::new()) };
    let root = tree(&bytes)?;
    Ok(root.nodes().filter(|node| node.name == "Relationship").filter_map(|node| {
        let id = node.attr("Id")?.to_string();
        let target = node.attr("Target")?;
        let external = node.attr("TargetMode").is_some_and(|mode| mode.eq_ignore_ascii_case("External"));
        let target = if external { target.trim().to_string() } else { resolve(folder, target) };
        Some((id, Rel { target, kind: node.attr("Type").unwrap_or_default().to_string(), external }))
    }).collect())
}

fn resolve(folder: &str, target: &str) -> String {
    let mut parts = if target.starts_with('/') { Vec::new() } else { folder.split('/').filter(|part| !part.is_empty()).collect::<Vec<_>>() };
    for segment in target.split('/') {
        match segment {
            "" | "." => {}
            ".." => { parts.pop(); }
            segment => parts.push(segment),
        }
    }
    parts.join("/")
}

#[derive(Debug, Default)]
struct ThemeFonts { minor: Option<String>, major: Option<String> }

impl ThemeFonts {
    fn read(root: &Node) -> Self {
        let face = |name: &str| root.find(name).and_then(|node| node.child("latin")).and_then(|latin| latin.attr("typeface")).filter(|face| !face.is_empty()).map(str::to_string);
        Self { minor: face("minorFont"), major: face("majorFont") }
    }
}

#[derive(Debug, Clone, Default)]
struct Run {
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    strike: Option<bool>,
    hidden: Option<bool>,
    half_points: Option<u16>,
    color: Option<Option<[u8; 3]>>,
    font: Option<String>,
}

impl Run {
    fn read(properties: &Node, theme: &ThemeFonts) -> Self {
        let fonts = properties.child("rFonts");
        let font = fonts.and_then(|fonts| fonts.attr("ascii").or(fonts.attr("hAnsi")).map(str::to_string)).or_else(|| {
            let slot = fonts.and_then(|fonts| fonts.attr("asciiTheme").or(fonts.attr("hAnsiTheme")))?;
            if slot.starts_with("major") { theme.major.clone() } else { theme.minor.clone() }
        });
        Self {
            bold: properties.on("b"),
            italic: properties.on("i"),
            underline: properties.child("u").map(|underline| underline.attr("val") != Some("none")),
            strike: properties.on("strike").or(properties.on("dstrike")),
            hidden: properties.on("vanish"),
            half_points: properties.val("sz").and_then(|value| value.parse::<u16>().ok()).map(|value| value.clamp(2, 3_276)),
            color: properties.val("color").map(hex_color).filter(Option::is_some).or(properties.val("color").filter(|value| *value == "auto").map(|_| None)),
            font,
        }
    }

    fn over(self, top: &Run) -> Self {
        Self {
            bold: top.bold.or(self.bold),
            italic: top.italic.or(self.italic),
            underline: top.underline.or(self.underline),
            strike: top.strike.or(self.strike),
            hidden: top.hidden.or(self.hidden),
            half_points: top.half_points.or(self.half_points),
            color: top.color.or(self.color),
            font: top.font.clone().or(self.font),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum LineRule { Auto, Exact, AtLeast }

#[derive(Debug, Clone, Default)]
struct Para {
    align: Option<Align>,
    indent: Option<f32>,
    right_indent: Option<f32>,
    before: Option<f32>,
    after: Option<f32>,
    line: Option<(f32, LineRule)>,
    page_break_before: Option<bool>,
    outline: Option<u8>,
    num_id: Option<String>,
    level: Option<u8>,
}

impl Para {
    fn read(properties: &Node) -> Self {
        let twips = |node: Option<&Node>, names: &[&str]| names.iter().find_map(|name| node?.attr(name)?.parse::<f32>().ok()).map(|value| value / 20.);
        let indent = properties.child("ind");
        let spacing = properties.child("spacing");
        let numbering = properties.child("numPr");
        Self {
            align: properties.val("jc").map(|value| match value {
                "center" => Align::Center,
                "right" | "end" => Align::Right,
                _ => Align::Left,
            }),
            indent: twips(indent, &["left", "start"]),
            right_indent: twips(indent, &["right", "end"]),
            before: twips(spacing, &["before"]),
            after: twips(spacing, &["after"]),
            line: spacing.and_then(|spacing| {
                let line = spacing.attr("line")?.parse::<f32>().ok()?;
                let rule = match spacing.attr("lineRule") { Some("exact") => LineRule::Exact, Some("atLeast") => LineRule::AtLeast, _ => LineRule::Auto };
                Some((line, rule))
            }),
            page_break_before: properties.on("pageBreakBefore"),
            outline: properties.val("outlineLvl").and_then(|value| value.parse().ok()),
            num_id: numbering.and_then(|numbering| numbering.val("numId")).map(str::to_string),
            level: numbering.and_then(|numbering| numbering.val("ilvl")).and_then(|value| value.parse().ok()),
        }
    }

    fn over(self, top: &Para) -> Self {
        Self {
            align: top.align.or(self.align),
            indent: top.indent.or(self.indent),
            right_indent: top.right_indent.or(self.right_indent),
            before: top.before.or(self.before),
            after: top.after.or(self.after),
            line: top.line.or(self.line),
            page_break_before: top.page_break_before.or(self.page_break_before),
            outline: top.outline.or(self.outline),
            num_id: top.num_id.clone().or(self.num_id),
            level: top.level.or(self.level),
        }
    }

    fn leading(&self, size: f32) -> f32 {
        match self.line {
            Some((line, LineRule::Auto)) => (1.2 * line / 240.).clamp(0.6, 6.),
            Some((line, LineRule::Exact)) => (line / 20. / size).clamp(0.3, 6.),
            Some((line, LineRule::AtLeast)) => (line / 20. / size).clamp(1.2, 6.),
            None => 1.2,
        }
    }
}

#[derive(Debug, Default)]
struct StyleDef { based_on: Option<String>, run: Run, para: Para, heading: Option<u8> }

#[derive(Debug, Default)]
struct Styles { run: Run, para: Para, defs: HashMap<String, StyleDef>, default_paragraph: Option<String> }

impl Styles {
    fn read(root: &Node, theme: &ThemeFonts) -> Self {
        let mut styles = Self::default();
        if let Some(defaults) = root.child("docDefaults") {
            if let Some(properties) = defaults.child("rPrDefault").and_then(|node| node.child("rPr")) { styles.run = Run::read(properties, theme); }
            if let Some(properties) = defaults.child("pPrDefault").and_then(|node| node.child("pPr")) { styles.para = Para::read(properties); }
        }
        for style in root.nodes().filter(|node| node.name == "style") {
            let Some(id) = style.attr("styleId") else { continue };
            if style.attr("type") == Some("paragraph") && style.attr("default").is_some_and(|value| matches!(value, "1" | "true" | "on")) {
                styles.default_paragraph = Some(id.to_string());
            }
            let heading = style.val("name").and_then(|name| name.to_ascii_lowercase().strip_prefix("heading ").and_then(|level| level.trim().parse::<u8>().ok())).filter(|level| (1..=6).contains(level));
            styles.defs.insert(id.to_string(), StyleDef {
                based_on: style.val("basedOn").map(str::to_string),
                run: style.child("rPr").map(|properties| Run::read(properties, theme)).unwrap_or_default(),
                para: style.child("pPr").map(Para::read).unwrap_or_default(),
                heading,
            });
        }
        styles
    }

    fn chain(&self, id: Option<&str>) -> Vec<&StyleDef> {
        let mut chain = Vec::new();
        let mut next = id;
        while let Some(def) = next.and_then(|id| self.defs.get(id)) {
            if chain.len() == MAX_STYLE_CHAIN { break; }
            chain.push(def);
            next = def.based_on.as_deref();
        }
        chain.reverse();
        chain
    }

    fn paragraph(&self, id: Option<&str>) -> ((Para, Option<u8>), Run) {
        let chain = self.chain(id.or(self.default_paragraph.as_deref()));
        let para = chain.iter().fold(self.para.clone(), |para, def| para.over(&def.para));
        let run = chain.iter().fold(self.run.clone(), |run, def| run.over(&def.run));
        let heading = chain.iter().rev().find_map(|def| def.heading);
        ((para, heading), run)
    }

    fn character(&self, id: &str) -> Run {
        self.chain(Some(id)).iter().fold(Run::default(), |run, def| run.over(&def.run))
    }
}

#[derive(Debug, Clone, Copy)]
struct Level { start: u64, bullet: bool }

#[derive(Debug, Default)]
struct Numbering { abstracts: HashMap<String, HashMap<u8, Level>>, nums: HashMap<String, (String, HashMap<u8, u64>)> }

impl Numbering {
    fn read(root: &Node) -> Self {
        let mut numbering = Self::default();
        for definition in root.nodes().filter(|node| node.name == "abstractNum") {
            let Some(id) = definition.attr("abstractNumId") else { continue };
            let levels = definition.nodes().filter(|node| node.name == "lvl").filter_map(|level| {
                let index = level.attr("ilvl")?.parse::<u8>().ok()?;
                let start = level.val("start").and_then(|value| value.parse().ok()).unwrap_or(1);
                Some((index, Level { start, bullet: matches!(level.val("numFmt"), Some("bullet" | "none")) }))
            }).collect();
            numbering.abstracts.insert(id.to_string(), levels);
        }
        for num in root.nodes().filter(|node| node.name == "num") {
            let (Some(id), Some(definition)) = (num.attr("numId"), num.val("abstractNumId")) else { continue };
            let overrides = num.nodes().filter(|node| node.name == "lvlOverride").filter_map(|item| {
                Some((item.attr("ilvl")?.parse().ok()?, item.val("startOverride")?.parse().ok()?))
            }).collect();
            numbering.nums.insert(id.to_string(), (definition.to_string(), overrides));
        }
        numbering
    }

    fn level(&self, id: &str, level: u8) -> Level {
        let Some((definition, overrides)) = self.nums.get(id) else { return Level { start: 1, bullet: true } };
        let mut found = self.abstracts.get(definition).and_then(|levels| levels.get(&level)).copied().unwrap_or(Level { start: 1, bullet: true });
        if let Some(start) = overrides.get(&level) { found.start = *start; }
        found
    }
}

fn embedded_fonts(package: &mut Package, path: &str) -> Result<Vec<EmbeddedFont>, ToolError> {
    let Some(bytes) = package.part(path)? else { return Ok(Vec::new()) };
    let table = tree(&bytes)?;
    let rels = rels(package, path)?;
    let mut fonts = Vec::new();
    for font in table.nodes().filter(|node| node.name == "font") {
        let Some(family) = font.attr("name") else { continue };
        for (tag, style) in [("embedRegular", FontStyle::Regular), ("embedBold", FontStyle::Bold), ("embedItalic", FontStyle::Italic), ("embedBoldItalic", FontStyle::BoldItalic)] {
            let Some(embed) = font.child(tag) else { continue };
            let Some(rel) = embed.attr("id").and_then(|id| rels.get(id)).filter(|rel| !rel.external) else { continue };
            let Some(mut data) = package.part(&rel.target)? else { continue };
            if let Some(key) = embed.attr("fontKey") { deobfuscate(&mut data, key); }
            fonts.push(EmbeddedFont { family: family.to_string(), style, data });
        }
    }
    Ok(fonts)
}

/// ECMA-376 font obfuscation: the first 32 bytes are XORed with the reversed bytes of the font key GUID.
pub fn deobfuscate(data: &mut [u8], key: &str) {
    let digits = key.chars().filter(char::is_ascii_hexdigit).collect::<String>();
    if digits.len() != 32 { return; }
    let mut bytes = (0..16).filter_map(|index| u8::from_str_radix(&digits[index * 2..index * 2 + 2], 16).ok()).collect::<Vec<_>>();
    bytes.reverse();
    for (index, byte) in data.iter_mut().take(32).enumerate() { *byte ^= bytes[index % 16]; }
}

struct Part { rels: HashMap<String, Rel>, band: bool }

#[derive(Debug, Clone, PartialEq)]
enum FieldKind { Page, Pages, Link(Option<String>), Other }

fn field_kind(instruction: &str) -> FieldKind {
    let mut words = instruction.split_whitespace();
    match words.next().map(str::to_ascii_uppercase).as_deref() {
        Some("PAGE") => FieldKind::Page,
        Some("NUMPAGES" | "SECTIONPAGES") => FieldKind::Pages,
        Some("HYPERLINK") => {
            let quoted = |text: &str| text.split('"').nth(1).map(str::to_string);
            let rest = instruction.trim_start().get(9..).unwrap_or_default();
            FieldKind::Link(match rest.find("\\l") {
                Some(index) => quoted(&rest[index..]).map(|anchor| format!("#{anchor}")),
                None => quoted(rest).or_else(|| rest.split_whitespace().next().map(str::to_string)).and_then(|target| safe_uri(&target)),
            })
        }
        _ => FieldKind::Other,
    }
}

struct Field { kind: FieldKind, instruction: String, result: bool, shown: bool }

struct Graphic { picture: Option<usize>, fill: Option<[u8; 3]>, line: Option<[u8; 3]>, blocks: Vec<Block> }

struct Converter<'a> {
    package: Package<'a>,
    theme: ThemeFonts,
    styles: Styles,
    numbering: Numbering,
    counters: HashMap<(String, u8), u64>,
    fonts: Families,
    body_half_points: u16,
    pictures: Vec<Picture>,
    omitted: Omitted,
    headings: usize,
    fields: Vec<Field>,
    section: Option<Node>,
    carry: Vec<Span>,
}

impl Converter<'_> {
    fn band(&mut self, section: &Node, reference: &str, body: &Part) -> Result<Vec<Block>, ToolError> {
        let references = section.nodes().filter(|node| node.name == reference).collect::<Vec<_>>();
        let chosen = references.iter().find(|node| node.attr("type") == Some("default")).or(references.first());
        let Some(rel) = chosen.and_then(|node| node.attr("id")).and_then(|id| body.rels.get(id)).filter(|rel| !rel.external) else { return Ok(Vec::new()) };
        let path = rel.target.clone();
        let Some(bytes) = self.package.part(&path)? else { return Ok(Vec::new()) };
        let part = Part { rels: rels(&mut self.package, &path)?, band: true };
        self.fields.clear();
        let mut out = Out::default();
        shared::walk(&bytes, |open| open.len() == 1, DAMAGED, |node| self.block(&node, &part, &mut out))?;
        self.fields.clear();
        Ok(out.finish())
    }

    fn block(&mut self, node: &Node, part: &Part, out: &mut Out) -> Result<(), ToolError> {
        match node.name.as_str() {
            "p" => self.paragraph(node, part, out)?,
            "tbl" => self.table(node, part, out)?,
            "sdt" => if let Some(content) = node.child("sdtContent") { for child in content.nodes() { self.block(child, part, out)?; } },
            "customXml" | "sdtContent" => for child in node.nodes() { self.block(child, part, out)?; },
            "AlternateContent" => self.alternate(node, out, |converter, branch, out| {
                for child in branch.nodes() { converter.block(child, part, out)?; }
                Ok(())
            })?,
            "sectPr" => self.section(node),
            _ => {}
        }
        Ok(())
    }

    /// Reads the preferred branch of a compatibility block, falling back to the stored alternative when the preferred one leaves content out.
    fn alternate<O: Clone>(&mut self, node: &Node, out: &mut O, mut each: impl FnMut(&mut Self, &Node, &mut O) -> Result<(), ToolError>) -> Result<(), ToolError> {
        let (choice, fallback) = (node.child("Choice"), node.child("Fallback"));
        let (Some(choice), Some(fallback)) = (choice, fallback) else {
            return match choice.or(fallback) { Some(branch) => each(self, branch, out), None => Ok(()) };
        };
        let (omitted, pictures) = (self.omitted, self.pictures.len());
        let mut tried = out.clone();
        each(self, choice, &mut tried)?;
        if self.omitted == omitted {
            *out = tried;
            return Ok(());
        }
        self.omitted = omitted;
        self.pictures.truncate(pictures);
        each(self, fallback, out)
    }

    fn text_box(&mut self, content: &Node, part: &Part) -> Result<Vec<Block>, ToolError> {
        let fields = std::mem::take(&mut self.fields);
        let carry = std::mem::take(&mut self.carry);
        let mut out = Out::default();
        let read = content.nodes().try_for_each(|child| self.block(child, part, &mut out));
        self.fields = fields;
        self.carry = carry;
        read?;
        Ok(out.finish())
    }

    fn section(&mut self, node: &Node) {
        if self.section.is_none() { self.section = Some(node.clone()); }
    }

    fn style(&mut self, run: &Run) -> Style {
        Style {
            bold: run.bold.unwrap_or(false),
            italic: run.italic.unwrap_or(false),
            code: false,
            strike: run.strike.unwrap_or(false),
            underline: run.underline.unwrap_or(false),
            family: self.fonts.index(run.font.as_deref()),
            half_points: run.half_points.unwrap_or(DEFAULT_HALF_POINTS),
            color: run.color.flatten(),
        }
    }

    fn collect(&mut self, paragraph: &Node, part: &Part) -> Result<(Collected, Para, Option<u8>, Run), ToolError> {
        let properties = paragraph.child("pPr");
        let ((mut para, heading), run) = self.styles.paragraph(properties.and_then(|properties| properties.val("pStyle")));
        if let Some(properties) = properties { para = para.over(&Para::read(properties)); }
        let mut collected = Collected::new();
        self.inline(paragraph, part, &run, None, &mut collected)?;
        if !self.carry.is_empty() {
            let carried = std::mem::take(&mut self.carry);
            collected.segments[0].splice(0..0, carried);
        }
        Ok((collected, para, heading, run))
    }

    fn paragraph(&mut self, node: &Node, part: &Part, out: &mut Out) -> Result<(), ToolError> {
        let (collected, para, style_heading, run) = self.collect(node, part)?;
        if para.page_break_before == Some(true) { out.page_break(); }
        let size = run.half_points.unwrap_or(DEFAULT_HALF_POINTS) as f32 / 2.;
        let heading = para.outline.filter(|level| *level < 6).map(|level| level + 1).or(style_heading);
        let list = para.num_id.clone().filter(|id| id != "0" && heading.is_none()).map(|id| (id, para.level.unwrap_or(0).min(8)));
        let Collected { segments, after, floats, bookmark } = collected;
        for float in floats { out.push(float); }
        let mark = node.child("pPr").and_then(|properties| properties.child("rPr"));
        let segments = if mark.is_some_and(|mark| mark.child("del").is_some() || mark.child("moveFrom").is_some()) {
            self.carry.extend(segments.into_iter().flatten());
            Vec::new()
        } else {
            segments
        };
        for (index, spans) in segments.into_iter().enumerate() {
            if index > 0 {
                out.page_break();
                if spans.iter().all(|span| span.text.trim().is_empty()) { continue; }
            }
            if let Some(level) = heading.filter(|_| !plain_text(&spans).is_empty()) {
                self.headings += 1;
                let anchor = bookmark.clone().filter(|_| index == 0).unwrap_or_else(|| format!("heading-{}", self.headings));
                out.push(Block::Heading { level, spans, anchor });
            } else if let Some((id, level)) = &list {
                let definition = self.numbering.level(id, *level);
                let start = (!definition.bullet).then(|| {
                    let value = *self.counters.entry((id.clone(), *level)).or_insert(definition.start);
                    self.counters.insert((id.clone(), *level), value + 1);
                    self.counters.retain(|(other, deeper), _| other != id || deeper <= level);
                    value
                });
                out.item(id, Item { level: *level, start, spans });
            } else {
                if index == 0 && !after.is_empty() && spans.iter().all(|span| span.text.trim().is_empty()) { continue; }
                out.push(Block::Formatted(Paragraph {
                    spans,
                    align: para.align.unwrap_or(Align::Left),
                    indent: para.indent.unwrap_or(0.).max(0.),
                    right_indent: para.right_indent.unwrap_or(0.).max(0.),
                    before: para.before.unwrap_or(0.).max(0.),
                    after: para.after.unwrap_or(0.).max(0.),
                    size,
                    leading: para.leading(size),
                }));
            }
        }
        for block in after { out.push(block); }
        if let Some(section) = node.child("pPr").and_then(|properties| properties.child("sectPr")) {
            self.section(section);
            if section.val("type") != Some("continuous") { out.page_break(); }
        }
        Ok(())
    }

    fn table(&mut self, node: &Node, part: &Part, out: &mut Out) -> Result<(), ToolError> {
        let mut head = Vec::new();
        let mut rows = Vec::new();
        let mut extra = Collected::default();
        let kept = node.nodes().filter(|node| node.name == "tr").filter(|row| !row.child("trPr").is_some_and(|properties| properties.child("del").is_some()));
        for (index, row) in kept.enumerate() {
            let header = index == 0 && row.child("trPr").and_then(|properties| properties.on("tblHeader")).unwrap_or(false);
            let mut cells = Vec::new();
            for cell in row.nodes().filter(|node| node.name == "tc") {
                let properties = cell.child("tcPr");
                if properties.is_some_and(|properties| properties.child("cellDel").is_some()) { continue; }
                let span = properties.and_then(|properties| properties.val("gridSpan")).and_then(|value| value.parse::<usize>().ok()).unwrap_or(1).clamp(1, 64);
                let merged = properties.and_then(|properties| properties.child("vMerge")).is_some_and(|merge| merge.attr("val") != Some("restart"));
                let mut spans = Vec::new();
                if !merged { self.cell(cell, part, &mut spans, &mut extra)?; }
                cells.push(spans);
                cells.extend(std::iter::repeat_with(Vec::new).take(span - 1));
            }
            if header { head = cells; } else { rows.push(cells); }
        }
        for float in extra.floats { out.push(float); }
        out.push(Block::Table { aligns: Vec::new(), head, rows });
        for block in extra.after { out.push(block); }
        Ok(())
    }

    fn cell(&mut self, node: &Node, part: &Part, spans: &mut Vec<Span>, extra: &mut Collected) -> Result<(), ToolError> {
        for child in node.nodes() {
            match child.name.as_str() {
                "p" => {
                    let (collected, ..) = self.collect(child, part)?;
                    let text = collected.segments.into_iter().flatten().collect::<Vec<_>>();
                    if !spans.is_empty() { spans.push(Span { text: "\n".into(), style: Style::default(), link: None }); }
                    spans.extend(text);
                    extra.after.extend(collected.after);
                    extra.floats.extend(collected.floats);
                }
                "tbl" | "tr" | "tc" | "sdt" | "sdtContent" | "customXml" => self.cell(child, part, spans, extra)?,
                _ => {}
            }
        }
        Ok(())
    }

    fn inline(&mut self, node: &Node, part: &Part, run: &Run, link: Option<&str>, out: &mut Collected) -> Result<(), ToolError> {
        for child in node.nodes() {
            match child.name.as_str() {
                "r" => self.run(child, part, run, link, out)?,
                "hyperlink" => {
                    let target = match (child.attr("id"), child.attr("anchor")) {
                        (Some(id), _) => part.rels.get(id).filter(|rel| rel.external).and_then(|rel| safe_uri(&rel.target)),
                        (None, Some(anchor)) => Some(format!("#{anchor}")),
                        (None, None) => None,
                    };
                    self.inline(child, part, run, target.as_deref().or(link), out)?;
                }
                "fldSimple" => {
                    let kind = field_kind(child.attr("instr").unwrap_or_default());
                    match kind {
                        FieldKind::Page | FieldKind::Pages if part.band => {
                            let style = self.style(run);
                            out.text(&sentinel(&kind), style, link);
                        }
                        FieldKind::Link(target) => self.inline(child, part, run, target.as_deref().or(link), out)?,
                        _ => self.inline(child, part, run, link, out)?,
                    }
                }
                "bookmarkStart" => {
                    if out.bookmark.is_none() { out.bookmark = child.attr("name").filter(|name| *name != "_GoBack").map(str::to_string); }
                }
                "ins" | "moveTo" | "smartTag" | "customXml" | "sdtContent" | "dir" | "bdo" => self.inline(child, part, run, link, out)?,
                "sdt" => if let Some(content) = child.child("sdtContent") { self.inline(content, part, run, link, out)?; },
                "AlternateContent" => self.alternate(child, out, |converter, branch, out| converter.inline(branch, part, run, link, out))?,
                _ => {}
            }
        }
        Ok(())
    }

    fn showing(&self, fields: &[Field], band: bool) -> bool {
        fields.iter().all(|field| field.result && !(band && matches!(field.kind, FieldKind::Page | FieldKind::Pages)))
    }

    fn run(&mut self, node: &Node, part: &Part, base: &Run, link: Option<&str>, out: &mut Collected) -> Result<(), ToolError> {
        let mut run = base.clone();
        let properties = node.child("rPr");
        if let Some(id) = properties.and_then(|properties| properties.val("rStyle")) { run = run.over(&self.styles.character(id)); }
        if let Some(properties) = properties { run = run.over(&Run::read(properties, &self.theme)); }
        let style = self.style(&run);
        let hidden = run.hidden == Some(true);
        self.run_content(node, part, style, hidden, link, out)
    }

    fn run_content(&mut self, node: &Node, part: &Part, style: Style, hidden: bool, link: Option<&str>, out: &mut Collected) -> Result<(), ToolError> {
        for child in node.nodes() {
            let field_link = self.fields.iter().rev().find_map(|field| match &field.kind { FieldKind::Link(target) if field.result => target.as_deref(), _ => None }).map(str::to_string);
            let link = field_link.as_deref().or(link);
            let visible = !hidden && self.showing(&self.fields, part.band);
            match child.name.as_str() {
                "t" if visible => out.text(&child.text(), style, link),
                "tab" | "ptab" if visible => out.text(" ", style, link),
                "cr" if visible => out.text("\n", style, link),
                "noBreakHyphen" if visible => out.text("-", style, link),
                "br" if visible => {
                    if child.attr("type") == Some("page") { out.segments.push(Vec::new()); } else { out.text("\n", style, link); }
                }
                "drawing" if visible => self.drawing(child, part, out)?,
                "pict" | "object" if visible => self.vml(child, part, out)?,
                "AlternateContent" => self.alternate(child, out, |converter, branch, out| converter.run_content(branch, part, style, hidden, link, out))?,
                "instrText" => if let Some(field) = self.fields.last_mut().filter(|field| !field.result) { field.instruction.push_str(&child.text()); },
                "fldChar" => match child.attr("fldCharType") {
                    Some("begin") => self.fields.push(Field { kind: FieldKind::Other, instruction: String::new(), result: false, shown: false }),
                    Some("separate") => {
                        let Some(mut field) = self.fields.pop() else { continue };
                        field.result = true;
                        field.kind = field_kind(&field.instruction);
                        self.page_field(&mut field, part, style, hidden, out);
                        self.fields.push(field);
                    }
                    Some("end") => {
                        let Some(mut field) = self.fields.pop() else { continue };
                        if !field.result { field.kind = field_kind(&field.instruction); }
                        self.page_field(&mut field, part, style, hidden, out);
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        Ok(())
    }

    fn page_field(&self, field: &mut Field, part: &Part, style: Style, hidden: bool, out: &mut Collected) {
        if field.shown || !part.band || !matches!(field.kind, FieldKind::Page | FieldKind::Pages) { return; }
        field.shown = true;
        if !hidden && self.showing(&self.fields, true) { out.text(&sentinel(&field.kind), style, None); }
    }

    fn drawing(&mut self, node: &Node, part: &Part, out: &mut Collected) -> Result<(), ToolError> {
        for frame in node.nodes().filter(|node| node.name == "inline" || node.name == "anchor") {
            let size = frame.child("extent").and_then(|extent| {
                let read = |name: &str| extent.attr(name)?.parse::<f32>().ok().map(|value| value / EMU_PER_POINT);
                Some((read("cx")?, read("cy")?))
            });
            let Some(graphic) = self.graphic(frame, part, size)? else { continue };
            if frame.name == "inline" {
                out.after.extend(graphic.picture.map(Block::Image));
                out.after.extend(graphic.blocks);
                continue;
            }
            let (width, height) = size.filter(|(width, height)| *width > 0. && *height > 0.).unwrap_or((DEFAULT_FLOAT_SIDE, DEFAULT_FLOAT_SIDE));
            let simple = frame.attr("simplePos").is_some_and(|value| matches!(value, "1" | "true" | "on"));
            let (x, y) = match frame.child("simplePos").filter(|_| simple) {
                Some(point) => {
                    let read = |name: &str| point.attr(name).and_then(|value| value.parse::<f32>().ok()).unwrap_or(0.) / EMU_PER_POINT;
                    (Position { frame: Frame::Page, place: Place::Offset(read("x")) }, Position { frame: Frame::Page, place: Place::Offset(read("y")) })
                }
                None => (anchor_position(frame.child("positionH")), anchor_position(frame.child("positionV"))),
            };
            let wrap = !frame.nodes().any(|node| node.name == "wrapNone");
            out.floats.push(Block::Float(Float { x, y, width, height, fill: graphic.fill, line: graphic.line, picture: graphic.picture, blocks: graphic.blocks, wrap }));
        }
        Ok(())
    }

    fn graphic(&mut self, frame: &Node, part: &Part, size: Option<(f32, f32)>) -> Result<Option<Graphic>, ToolError> {
        let Some(data) = frame.find("graphicData") else {
            self.omitted.objects += 1;
            return Ok(None);
        };
        let kind = data.attr("uri").and_then(|uri| uri.rsplit('/').next()).unwrap_or_default();
        match kind {
            "chart" | "chartex" => self.omitted.charts += 1,
            "diagram" => self.omitted.diagrams += 1,
            "wordprocessingShape" => {
                let Some(shape) = data.child("wsp") else { return Ok(None) };
                let (fill, line) = shape_paint(shape);
                let blocks = match shape.find("txbxContent") { Some(content) => self.text_box(content, part)?, None => Vec::new() };
                return Ok(Some(Graphic { picture: None, fill, line, blocks }));
            }
            "wordprocessingGroup" | "wordprocessingCanvas" => {
                let mut blocks = Vec::new();
                for content in descendants(data, "txbxContent") { blocks.extend(self.text_box(content, part)?); }
                let picture = match data.find("blip") { Some(blip) => self.picture(blip.attr("embed"), part, None)?, None => None };
                return Ok(Some(Graphic { picture, fill: None, line: None, blocks }));
            }
            _ => match data.find("blip") {
                Some(blip) => match self.picture(blip.attr("embed"), part, size)? {
                    Some(index) => return Ok(Some(Graphic { picture: Some(index), fill: None, line: None, blocks: Vec::new() })),
                    None => self.omitted.pictures += 1,
                },
                None => self.omitted.objects += 1,
            },
        }
        Ok(None)
    }

    fn picture(&mut self, id: Option<&str>, part: &Part, size: Option<(f32, f32)>) -> Result<Option<usize>, ToolError> {
        let Some(rel) = id.and_then(|id| part.rels.get(id)).filter(|rel| !rel.external).cloned() else { return Ok(None) };
        let Some(picture) = self.package.part(&rel.target)?.and_then(|bytes| Picture::from_bytes(&bytes, size).ok()) else { return Ok(None) };
        self.pictures.push(picture);
        Ok(Some(self.pictures.len() - 1))
    }

    /// Legacy VML drawings and OLE objects: only the preview image and text box text they store are used.
    fn vml(&mut self, node: &Node, part: &Part, out: &mut Collected) -> Result<(), ToolError> {
        let shape = node.nodes().find(|child| child.attr("style").is_some());
        let style = shape.and_then(|shape| shape.attr("style")).map(vml_style).unwrap_or_default();
        let length = |name: &str| style.get(name).and_then(|value| css_points(value));
        let size = length("width").zip(length("height"));
        let picture = match node.find("imagedata") {
            Some(image) => match self.picture(image.attr("id").or(image.attr("relid")), part, size)? {
                Some(index) => Some(index),
                None => {
                    self.omitted.pictures += 1;
                    return Ok(());
                }
            },
            None => None,
        };
        let blocks = match node.find("txbxContent") { Some(content) => self.text_box(content, part)?, None => Vec::new() };
        if picture.is_none() && blocks.is_empty() {
            self.omitted.objects += 1;
            return Ok(());
        }
        if style.get("position").map(String::as_str) != Some("absolute") {
            out.after.extend(picture.map(Block::Image));
            out.after.extend(blocks);
            return Ok(());
        }
        let frame = |name: &str| match style.get(name).map(String::as_str) {
            Some("page") => Frame::Page,
            Some("margin") => Frame::Margin,
            _ => Frame::Text,
        };
        let offset = |name: &str| Place::Offset(length(name).unwrap_or(0.));
        let shape = shape.expect("an absolute position comes from a styled shape");
        let colour = |name: &str, switch: &str| (!matches!(shape.attr(switch), Some("f" | "false"))).then(|| shape.attr(name).and_then(|value| hex_color(value.trim_start_matches('#')))).flatten();
        let fill = if picture.is_some() { None } else { colour("fillcolor", "filled").or_else(|| (!matches!(shape.attr("filled"), Some("f" | "false"))).then_some([255, 255, 255])) };
        let line = if picture.is_some() { None } else { colour("strokecolor", "stroked").or_else(|| (!matches!(shape.attr("stroked"), Some("f" | "false"))).then_some([0, 0, 0])) };
        let (width, height) = size.unwrap_or((DEFAULT_FLOAT_SIDE, DEFAULT_FLOAT_SIDE));
        let wrap = node.find("wrap").and_then(|wrap| wrap.attr("type")) != Some("none");
        out.floats.push(Block::Float(Float {
            x: Position { frame: frame("mso-position-horizontal-relative"), place: offset("margin-left") },
            y: Position { frame: frame("mso-position-vertical-relative"), place: offset("margin-top") },
            width,
            height,
            fill,
            line,
            picture,
            blocks,
            wrap,
        }));
        Ok(())
    }
}

fn descendants<'n>(node: &'n Node, name: &str) -> Vec<&'n Node> {
    let mut found = Vec::new();
    for child in node.nodes() {
        if child.name == name { found.push(child); } else { found.extend(descendants(child, name)); }
    }
    found
}

fn anchor_position(node: Option<&Node>) -> Position {
    let frame = match node.and_then(|node| node.attr("relativeFrom")) {
        Some("page") => Frame::Page,
        Some("margin" | "leftMargin" | "rightMargin" | "topMargin" | "bottomMargin" | "insideMargin" | "outsideMargin") => Frame::Margin,
        _ => Frame::Text,
    };
    let place = node.and_then(|node| match node.child("posOffset") {
        Some(offset) => offset.text().trim().parse::<f32>().ok().map(|value| Place::Offset(value / EMU_PER_POINT)),
        None => node.child("align").map(|align| match align.text().trim() {
            "center" => Place::Center,
            "right" | "bottom" | "outside" => Place::End,
            _ => Place::Start,
        }),
    });
    Position { frame, place: place.unwrap_or(Place::Offset(0.)) }
}

/// Explicit colours from the shape properties, else a neutral stand-in when the shape style asks for a fill or outline.
fn shape_paint(shape: &Node) -> (Option<[u8; 3]>, Option<[u8; 3]>) {
    let properties = shape.child("spPr");
    let style = shape.child("style");
    let paint = |explicit: Option<&Node>, reference: Option<&Node>, stand_in: [u8; 3]| -> Option<[u8; 3]> {
        if let Some(explicit) = explicit {
            if explicit.child("noFill").is_some() { return None; }
            if let Some(fill) = explicit.child("solidFill") {
                return Some(fill.child("srgbClr").and_then(|colour| colour.attr("val")).and_then(hex_color).unwrap_or(stand_in));
            }
        }
        reference.filter(|reference| reference.attr("idx").is_some_and(|index| index != "0")).map(|reference| {
            reference.child("srgbClr").and_then(|colour| colour.attr("val")).and_then(hex_color).unwrap_or(stand_in)
        })
    };
    let fill = paint(properties, style.and_then(|style| style.child("fillRef")), SHAPE_FILL);
    let line = paint(properties.and_then(|properties| properties.child("ln")), style.and_then(|style| style.child("lnRef")), SHAPE_LINE);
    (fill, line)
}

fn vml_style(style: &str) -> HashMap<String, String> {
    style.split(';').filter_map(|declaration| {
        let (key, value) = declaration.split_once(':')?;
        Some((key.trim().to_ascii_lowercase(), value.trim().to_ascii_lowercase()))
    }).collect()
}

fn sentinel(kind: &FieldKind) -> String {
    if *kind == FieldKind::Pages { PAGE_COUNT.to_string() } else { PAGE_NUMBER.to_string() }
}

#[cfg(test)]
pub mod fixtures {
    use std::io::{Cursor, Write};

    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    pub const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office" xmlns:w10="urn:schemas-microsoft-com:office:word""#;
    const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

    /// Builds a DOCX package from a body, document relationships `(id, type suffix, target, external)` and extra parts.
    pub fn docx(body: &str, rels: &[(&str, &str, &str, bool)], parts: &[(&str, &[u8])]) -> Vec<u8> {
        let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
        let mut add = |name: &str, data: &[u8]| {
            archive.start_file(name, SimpleFileOptions::default()).unwrap();
            archive.write_all(data).unwrap();
        };
        add("[Content_Types].xml", br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#);
        add("_rels/.rels", format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}/officeDocument" Target="word/document.xml"/></Relationships>"#).as_bytes());
        add("word/document.xml", format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {W}><w:body>{body}</w:body></w:document>"#).as_bytes());
        let rels = rels.iter().map(|(id, kind, target, external)| {
            let mode = if *external { r#" TargetMode="External""# } else { "" };
            format!(r#"<Relationship Id="{id}" Type="{REL}/{kind}" Target="{target}"{mode}/>"#)
        }).collect::<String>();
        add("word/_rels/document.xml.rels", format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rels}</Relationships>"#).as_bytes());
        for (name, data) in parts { add(name, data); }
        archive.finish().unwrap().into_inner()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::contracts::{ErrorKind, LimitKind};

    fn exceeded() -> ToolError { ToolError::limit_exceeded(LimitKind::Decompression, "over") }

    #[test]
    fn the_expansion_budget_covers_every_part_read() {
        let body = format!(r#"<w:p><w:r><w:t>{}</w:t></w:r></w:p>"#, "x".repeat(4_000));
        let bytes = fixtures::docx(&body, &[], &[]);
        assert!(parse(&bytes, 1_000_000, exceeded()).is_ok());
        let error = parse(&bytes, 3_000, exceeded()).err().unwrap();
        assert!(matches!(error.kind, ErrorKind::LimitExceeded { .. }), "{error:?}");
    }

    #[test]
    fn unbalanced_or_deeply_nested_xml_is_damaged() {
        let error = parse(&fixtures::docx("<w:p><w:r>", &[], &[]), 1_000_000, exceeded()).err().unwrap();
        assert_eq!(error.message, DAMAGED);
        let deep = "<w:sdt><w:sdtContent>".repeat(200) + &"</w:sdtContent></w:sdt>".repeat(200);
        assert_eq!(parse(&fixtures::docx(&deep, &[], &[]), 1_000_000, exceeded()).err().unwrap().message, DAMAGED);
    }

    #[test]
    fn relationship_targets_resolve_inside_the_package_and_links_keep_safe_schemes() {
        assert_eq!(resolve("word", "media/a.png"), "word/media/a.png");
        assert_eq!(resolve("word", "../customXml/item.xml"), "customXml/item.xml");
        assert_eq!(resolve("word", "/word/media/b.png"), "word/media/b.png");
        assert_eq!(safe_uri("https://example.com"), Some("https://example.com".into()));
        assert_eq!(safe_uri("file:///etc/passwd"), None);
        assert_eq!(field_kind(r#" HYPERLINK "https://example.com/x" "#), FieldKind::Link(Some("https://example.com/x".into())));
        assert_eq!(field_kind(r#"HYPERLINK \l "_Toc1""#), FieldKind::Link(Some("#_Toc1".into())));
        assert_eq!(field_kind(" PAGE \\* MERGEFORMAT "), FieldKind::Page);
    }

    #[test]
    fn deobfuscation_reverses_the_font_key() {
        let key = "{00112233-4455-6677-8899-AABBCCDDEEFF}";
        let mut data = vec![0u8; 40];
        deobfuscate(&mut data, key);
        assert_eq!(&data[..16], &[0xFF, 0xEE, 0xDD, 0xCC, 0xBB, 0xAA, 0x99, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11, 0x00]);
        assert_eq!(&data[16..32], &data[..16]);
        assert!(data[32..].iter().all(|byte| *byte == 0));
    }
}
