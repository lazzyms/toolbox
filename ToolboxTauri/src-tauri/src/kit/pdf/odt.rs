use std::collections::HashMap;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;

use crate::kit::contracts::ToolError;
use crate::kit::pdf::font_resolver::{EmbeddedFont, FontStyle};
use crate::kit::pdf::layout::{plain_text, Align, Block, Float, Frame, Omitted, PageSetup, Paragraph, Picture, Place, Position, Span, Style, PAGE_COUNT, PAGE_NUMBER};
use crate::kit::pdf::wordprocessing::{self as shared, css_points, hex_color, safe_uri, Child, Collected, Families, Item, Node, Out, Package, Parsed, DEFAULT_FLOAT_SIDE, SHAPE_FILL, SHAPE_LINE};

pub const DAMAGED: &str = "This OpenDocument text file is damaged or incomplete, so no PDF was made.";
const CONTENT: &str = "content.xml";
const CHART: &str = "application/vnd.oasis.opendocument.chart";
const DEFAULT_SIZE: f32 = 12.;
const MAX_STYLE_CHAIN: usize = 16;
const MAX_REPEAT: usize = 64;
const MAX_SPACES: usize = 1_000;
const DRAWINGS: &[&str] = &["frame", "a", "g", "custom-shape", "rect", "ellipse", "circle", "regular-polygon", "line", "polyline", "polygon", "path", "connector", "measure", "control", "caption"];
const SKIPPED: &[&str] = &[
    "note", "annotation", "annotation-end", "tracked-changes", "change", "change-start", "change-end", "bookmark-end", "number",
    "reference-mark", "reference-mark-start", "reference-mark-end", "toc-mark", "toc-mark-start", "toc-mark-end",
    "alphabetical-index-mark", "alphabetical-index-mark-start", "alphabetical-index-mark-end", "user-index-mark", "user-index-mark-start", "user-index-mark-end",
    "soft-page-break", "hidden-paragraph",
];

type Props = HashMap<String, String>;

fn damaged() -> ToolError { ToolError::invalid_input(DAMAGED) }

fn tree(bytes: &[u8]) -> Result<Node, ToolError> { shared::tree(bytes, DAMAGED) }

fn in_body(open: &[Node]) -> bool { open.len() == 3 && open[1].name == "body" && open[2].name == "text" }

/// Reads an ODT package. `budget` caps the bytes expanded across every part read; going over returns `exceeded`, and a password-protected package returns `locked`.
pub fn parse(bytes: &[u8], budget: u64, exceeded: ToolError, locked: ToolError) -> Result<Parsed, ToolError> {
    let mut package = Package::open(bytes, budget, exceeded, DAMAGED)?;
    let mut media = HashMap::new();
    if let Some(manifest) = package.part("META-INF/manifest.xml")? {
        let manifest = tree(&manifest)?;
        if manifest.find("encryption-data").is_some() { return Err(locked); }
        for entry in manifest.nodes().filter(|node| node.name == "file-entry") {
            if let (Some(path), Some(kind)) = (entry.attr("full-path"), entry.attr("media-type")) { media.insert(path.trim_end_matches('/').to_string(), kind.to_string()); }
        }
    }

    let mut styles = Styles::default();
    if let Some(bytes) = package.part("styles.xml")? { styles.read(&tree(&bytes)?); }
    let content = package.part(CONTENT)?.ok_or_else(damaged)?;
    let head = shared::walk(&content, in_body, DAMAGED, |_| Ok(()))?;
    if head.name != "document-content" || head.child("body").and_then(|body| body.child("text")).is_none() { return Err(damaged()); }
    styles.read(&head);
    let embedded = styles.embedded(&mut package)?;

    let base = styles.paragraph(Some("Standard"));
    let mut converter = Converter {
        fonts: Families { body: styles.font(&base.text), others: Vec::new() },
        body_size: font_size(&base.text),
        package,
        styles,
        media,
        pictures: Vec::new(),
        omitted: Omitted::default(),
        headings: 0,
        lists: 0,
        band: false,
        master: None,
    };
    let mut out = Out::default();
    shared::walk(&content, in_body, DAMAGED, |node| converter.block(&node, &mut out))?;
    let blocks = out.finish();

    let masters = std::mem::take(&mut converter.styles.masters);
    let master = masters.iter().find(|master| master.attr("name").is_some() && master.attr("name") == converter.master.as_deref())
        .or_else(|| masters.iter().find(|master| master.attr("name") == Some("Standard")))
        .or(masters.first());
    let band = |converter: &mut Converter, name: &str| -> Result<Vec<Block>, ToolError> {
        let Some(content) = master.and_then(|master| master.child(name)).filter(|band| band.attr("display") != Some("false")) else { return Ok(Vec::new()) };
        converter.band = true;
        let mut out = Out::default();
        let read = content.nodes().try_for_each(|child| converter.block(child, &mut out));
        converter.band = false;
        read?;
        Ok(out.finish())
    };
    let header = band(&mut converter, "header")?;
    let footer = band(&mut converter, "footer")?;

    let layout = master.and_then(|master| master.attr("page-layout-name")).and_then(|name| converter.styles.layouts.get(name));
    let page = layout.and_then(|layout| layout.child("page-layout-properties"));
    let length = |node: Option<&Node>, name: &str| node.and_then(|node| node.attr(name).or(node.attr("margin"))).and_then(css_points);
    let band_height = |style: &str, spacing: &str| {
        let properties = layout.and_then(|layout| layout.child(style)).and_then(|style| style.child("header-footer-properties"));
        properties.and_then(|properties| properties.attr("min-height").or(properties.attr("height"))).and_then(css_points).unwrap_or(0.)
            + properties.and_then(|properties| properties.attr(spacing)).and_then(css_points).unwrap_or(0.)
    };
    let (top, bottom) = (length(page, "margin-top"), length(page, "margin-bottom"));
    let header_distance = top.filter(|_| !header.is_empty());
    let footer_distance = bottom.filter(|_| !footer.is_empty());
    let page = PageSetup {
        width: page.and_then(|page| page.attr("page-width")).and_then(css_points),
        height: page.and_then(|page| page.attr("page-height")).and_then(css_points),
        left: length(page, "margin-left"),
        right: length(page, "margin-right"),
        top: top.map(|top| top + if header.is_empty() { 0. } else { band_height("header-style", "margin-bottom") }),
        bottom: bottom.map(|bottom| bottom + if footer.is_empty() { 0. } else { band_height("footer-style", "margin-top") }),
    };

    let (title, author) = match converter.package.part("meta.xml")? {
        Some(bytes) => {
            let meta = tree(&bytes)?;
            let field = |name: &str| meta.find(name).map(|node| node.text().trim().to_string()).filter(|text| !text.is_empty());
            (field("title"), field("initial-creator").or_else(|| field("creator")))
        }
        None => (None, None),
    };

    Ok(Parsed {
        blocks,
        header,
        footer,
        header_distance,
        footer_distance,
        pictures: converter.pictures,
        families: converter.fonts.others,
        body_family: converter.fonts.body,
        body_size: converter.body_size,
        embedded,
        title,
        author,
        page,
        omitted: converter.omitted,
    })
}

#[derive(Debug, Default)]
struct StyleDef { parent: Option<String>, text: Props, paragraph: Props, graphic: Props, master: Option<String>, outline: Option<u8> }

#[derive(Debug, Clone, Copy)]
struct Level { bullet: bool, start: u64 }

#[derive(Debug, Default)]
struct Styles {
    defs: HashMap<String, StyleDef>,
    defaults: HashMap<String, StyleDef>,
    lists: HashMap<String, HashMap<u8, Level>>,
    faces: HashMap<String, (String, Vec<Node>)>,
    layouts: HashMap<String, Node>,
    masters: Vec<Node>,
}

struct ParaStyle { text: Props, para: Props, outline: Option<u8>, master: Option<String> }

fn unquote(value: &str) -> String { value.trim().trim_matches(|character| character == '\'' || character == '"').to_string() }

fn properties(node: &Node) -> Props { node.attrs.iter().cloned().collect() }

/// Later text properties win; a percentage font size scales the size it overrides.
fn merge_text(into: &mut Props, from: &Props) {
    for (key, value) in from {
        let value = match value.strip_suffix('%').and_then(|percent| percent.trim().parse::<f32>().ok()).filter(|_| key == "font-size") {
            Some(percent) => format!("{}pt", font_size(into) * percent / 100.),
            None => value.clone(),
        };
        into.insert(key.clone(), value);
    }
}

fn font_size(text: &Props) -> f32 {
    text.get("font-size").and_then(|value| css_points(value)).filter(|size| *size > 0.).unwrap_or(DEFAULT_SIZE).clamp(1., 1_638.)
}

impl Styles {
    fn read(&mut self, root: &Node) {
        for section in root.nodes() {
            match section.name.as_str() {
                "font-face-decls" => for face in section.nodes().filter(|node| node.name == "font-face") {
                    let Some(name) = face.attr("name") else { continue };
                    let family = face.attr("font-family").map(unquote).filter(|family| !family.is_empty()).unwrap_or_else(|| name.to_string());
                    let sources = face.child("font-face-src").map(|source| source.nodes().filter(|node| node.name == "font-face-uri").cloned().collect()).unwrap_or_default();
                    self.faces.insert(name.to_string(), (family, sources));
                },
                "styles" | "automatic-styles" => for style in section.nodes() { self.style(style); },
                "master-styles" => self.masters.extend(section.nodes().filter(|node| node.name == "master-page").cloned()),
                _ => {}
            }
        }
    }

    fn style(&mut self, node: &Node) {
        let definition = || {
            let mut definition = StyleDef {
                parent: node.attr("parent-style-name").map(str::to_string),
                master: node.attr("master-page-name").filter(|name| !name.is_empty()).map(str::to_string),
                outline: node.attr("default-outline-level").and_then(|level| level.parse().ok()),
                ..StyleDef::default()
            };
            for child in node.nodes() {
                match child.name.as_str() {
                    "text-properties" => definition.text = properties(child),
                    "paragraph-properties" => definition.paragraph = properties(child),
                    "graphic-properties" => definition.graphic = properties(child),
                    _ => {}
                }
            }
            definition
        };
        let family = node.attr("family").unwrap_or_default();
        match node.name.as_str() {
            "style" => if let Some(name) = node.attr("name") { self.defs.insert(format!("{family}/{name}"), definition()); },
            "default-style" => { self.defaults.insert(family.to_string(), definition()); }
            "list-style" => if let Some(name) = node.attr("name") {
                let levels = node.nodes().filter_map(|level| {
                    let index = level.attr("level")?.parse::<u8>().ok()?;
                    let numbered = level.name == "list-level-style-number" && level.attr("num-format").is_some_and(|format| !format.is_empty());
                    Some((index, Level { bullet: !numbered, start: level.attr("start-value").and_then(|value| value.parse().ok()).unwrap_or(1) }))
                }).collect();
                self.lists.insert(name.to_string(), levels);
            },
            "page-layout" => if let Some(name) = node.attr("name") { self.layouts.insert(name.to_string(), node.clone()); },
            _ => {}
        }
    }

    fn chain(&self, family: &str, name: Option<&str>) -> Vec<&StyleDef> {
        let mut chain = Vec::new();
        let mut next = name.map(str::to_string);
        while let Some(def) = next.and_then(|name| self.defs.get(&format!("{family}/{name}"))) {
            if chain.len() == MAX_STYLE_CHAIN { break; }
            chain.push(def);
            next = def.parent.clone();
        }
        chain.reverse();
        chain
    }

    fn paragraph(&self, name: Option<&str>) -> ParaStyle {
        let chain = self.chain("paragraph", name);
        let mut text = Props::new();
        let mut para = Props::new();
        for def in self.defaults.get("paragraph").into_iter().chain(chain.iter().copied()) {
            merge_text(&mut text, &def.text);
            para.extend(def.paragraph.clone());
        }
        ParaStyle { text, para, outline: chain.iter().rev().find_map(|def| def.outline), master: chain.last().and_then(|def| def.master.clone()) }
    }

    fn text(&self, base: &Props, name: Option<&str>) -> Props {
        let mut text = base.clone();
        for def in self.chain("text", name) { merge_text(&mut text, &def.text); }
        text
    }

    fn graphic(&self, name: Option<&str>) -> Props {
        let mut graphic = self.defaults.get("graphic").map(|def| def.graphic.clone()).unwrap_or_default();
        for def in self.chain("graphic", name) { graphic.extend(def.graphic.clone()); }
        graphic
    }

    fn level(&self, list: Option<&str>, level: u8) -> Level {
        list.and_then(|name| self.lists.get(name)).and_then(|levels| levels.get(&(level + 1))).copied().unwrap_or(Level { bullet: true, start: 1 })
    }

    fn font(&self, text: &Props) -> Option<String> {
        text.get("font-name").map(|name| self.faces.get(name).map(|(family, _)| family.clone()).unwrap_or_else(|| name.clone()))
            .or_else(|| text.get("font-family").map(|family| unquote(family)))
            .filter(|family| !family.is_empty())
    }

    fn embedded(&self, package: &mut Package) -> Result<Vec<EmbeddedFont>, ToolError> {
        let mut faces = self.faces.values().collect::<Vec<_>>();
        faces.sort_by(|left, right| left.0.cmp(&right.0));
        let mut fonts = Vec::new();
        for (family, sources) in faces {
            for source in sources {
                let Some(path) = source.attr("href").and_then(internal) else { continue };
                let Some(data) = package.part(&path)? else { continue };
                let bold = source.attr("font-weight").is_some_and(|weight| weight == "bold" || weight.parse::<u16>().is_ok_and(|weight| weight >= 600));
                let italic = matches!(source.attr("font-style"), Some("italic" | "oblique"));
                fonts.push(EmbeddedFont { family: family.clone(), style: FontStyle::new(bold, italic), data });
            }
        }
        Ok(fonts)
    }
}

/// A package path for a link that stays inside the file; anything pointing outside it is never read.
fn internal(href: &str) -> Option<String> {
    let path = href.trim().trim_start_matches("./");
    let outside = path.is_empty() || path.starts_with('/') || path.starts_with("..") || path.contains(':') || path.split('/').any(|segment| segment == "..");
    (!outside).then(|| path.to_string())
}

fn link_target(href: &str) -> Option<String> {
    match href.trim().strip_prefix('#') {
        Some(anchor) => (!anchor.is_empty()).then(|| format!("#{anchor}")),
        None => safe_uri(href),
    }
}

struct Converter<'a> {
    package: Package<'a>,
    styles: Styles,
    media: HashMap<String, String>,
    fonts: Families,
    body_size: f32,
    pictures: Vec<Picture>,
    omitted: Omitted,
    headings: usize,
    lists: usize,
    band: bool,
    master: Option<String>,
}

impl Converter<'_> {
    fn block(&mut self, node: &Node, out: &mut Out) -> Result<(), ToolError> {
        match node.name.as_str() {
            "p" | "h" => self.paragraph(node, out)?,
            "list" => {
                self.lists += 1;
                let id = format!("list-{}", self.lists);
                self.list(node, &id, 0, None, out)?;
            }
            "table" => self.table(node, out)?,
            "section" | "index-body" | "index-title" | "table-of-content" | "illustration-index" | "table-index" | "object-index" | "user-index" | "alphabetical-index" | "bibliography" | "numbered-paragraph" => {
                for child in node.nodes() { self.block(child, out)?; }
            }
            name if DRAWINGS.contains(&name) => {
                let mut collected = Collected::new();
                self.drawing(node, &mut collected)?;
                for block in collected.floats.into_iter().chain(collected.after) { out.push(block); }
            }
            _ => {}
        }
        Ok(())
    }

    fn collect(&mut self, node: &Node) -> Result<(Collected, ParaStyle), ToolError> {
        let style = self.styles.paragraph(node.attr("style-name"));
        let mut collected = Collected::new();
        self.inline(node, &style.text, None, &mut collected)?;
        Ok((collected, style))
    }

    fn paragraph(&mut self, node: &Node, out: &mut Out) -> Result<(), ToolError> {
        let (collected, style) = self.collect(node)?;
        if !self.band {
            if let Some(master) = &style.master {
                if self.master.is_none() { self.master = Some(master.clone()); } else { out.page_break(); }
            }
        }
        if style.para.get("break-before").map(String::as_str) == Some("page") { out.page_break(); }
        let Collected { mut segments, after, floats, bookmark } = collected;
        let spans = segments.swap_remove(0);
        for float in floats { out.push(float); }
        let heading = (node.name == "h").then(|| node.attr("outline-level").and_then(|level| level.parse::<u8>().ok()).or(style.outline).unwrap_or(1).clamp(1, 6));
        if let Some(level) = heading.filter(|_| !plain_text(&spans).trim().is_empty()) {
            self.headings += 1;
            let anchor = bookmark.unwrap_or_else(|| format!("heading-{}", self.headings));
            out.push(Block::Heading { level, spans, anchor });
        } else if after.is_empty() || !spans.iter().all(|span| span.text.trim().is_empty()) {
            out.push(Block::Formatted(formatted(spans, &style)));
        }
        for block in after { out.push(block); }
        if style.para.get("break-after").map(String::as_str) == Some("page") { out.page_break(); }
        Ok(())
    }

    fn list(&mut self, node: &Node, id: &str, level: u8, inherited: Option<&str>, out: &mut Out) -> Result<(), ToolError> {
        let list_style = node.attr("style-name").or(inherited).map(str::to_string);
        let definition = self.styles.level(list_style.as_deref(), level);
        let mut counter = None;
        for item in node.nodes().filter(|node| node.name == "list-item" || node.name == "list-header") {
            let mut spans: Option<Vec<Span>> = None;
            let mut trailing = Vec::new();
            let mut number = None;
            if item.name == "list-item" && !definition.bullet {
                let value = item.attr("start-value").and_then(|value| value.parse().ok()).unwrap_or(counter.unwrap_or(definition.start));
                counter = Some(value + 1);
                number = Some(value);
            }
            let flush = |spans: &mut Option<Vec<Span>>, trailing: &mut Vec<Block>, out: &mut Out| {
                if let Some(spans) = spans.take() { out.item(id, Item { level: level.min(8), start: number, spans }); }
                for block in trailing.drain(..) { out.push(block); }
            };
            for child in item.nodes() {
                match child.name.as_str() {
                    "p" => {
                        let (collected, _) = self.collect(child)?;
                        let text = collected.segments.into_iter().flatten();
                        match &mut spans {
                            Some(existing) => {
                                existing.push(Span { text: "\n".into(), style: Style::default(), link: None });
                                existing.extend(text);
                            }
                            None => spans = Some(text.collect()),
                        }
                        trailing.extend(collected.floats.into_iter().chain(collected.after));
                    }
                    "list" => {
                        flush(&mut spans, &mut trailing, out);
                        self.list(child, id, level.saturating_add(1), list_style.as_deref(), out)?;
                    }
                    _ => {
                        flush(&mut spans, &mut trailing, out);
                        self.block(child, out)?;
                    }
                }
            }
            flush(&mut spans, &mut trailing, out);
        }
        Ok(())
    }

    fn table(&mut self, node: &Node, out: &mut Out) -> Result<(), ToolError> {
        let mut head = Vec::new();
        let mut rows = Vec::new();
        let mut extra = Collected::default();
        self.rows(node, false, &mut head, &mut rows, &mut extra)?;
        for float in extra.floats { out.push(float); }
        out.push(Block::Table { aligns: Vec::new(), head, rows });
        for block in extra.after { out.push(block); }
        Ok(())
    }

    fn rows(&mut self, node: &Node, header: bool, head: &mut Vec<Vec<Span>>, rows: &mut Vec<Vec<Vec<Span>>>, extra: &mut Collected) -> Result<(), ToolError> {
        for child in node.nodes() {
            match child.name.as_str() {
                "table-header-rows" => self.rows(child, true, head, rows, extra)?,
                "table-rows" | "table-row-group" => self.rows(child, header, head, rows, extra)?,
                "table-row" => {
                    let mut cells = Vec::new();
                    for cell in child.nodes().filter(|node| node.name == "table-cell" || node.name == "covered-table-cell") {
                        let mut spans = Vec::new();
                        if cell.name == "table-cell" { self.cell(cell, &mut spans, extra)?; }
                        let repeat = cell.attr("number-columns-repeated").and_then(|value| value.parse::<usize>().ok()).unwrap_or(1).clamp(1, MAX_REPEAT);
                        cells.extend(std::iter::repeat_n(spans, repeat));
                    }
                    let empty = cells.iter().all(Vec::is_empty);
                    let repeat = if empty { 1 } else { child.attr("number-rows-repeated").and_then(|value| value.parse::<usize>().ok()).unwrap_or(1).clamp(1, MAX_REPEAT) };
                    for _ in 0..repeat {
                        if header && head.is_empty() { *head = cells.clone(); } else { rows.push(cells.clone()); }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn cell(&mut self, node: &Node, spans: &mut Vec<Span>, extra: &mut Collected) -> Result<(), ToolError> {
        for child in node.nodes() {
            match child.name.as_str() {
                "p" | "h" => {
                    let (collected, _) = self.collect(child)?;
                    if !spans.is_empty() { spans.push(Span { text: "\n".into(), style: Style::default(), link: None }); }
                    spans.extend(collected.segments.into_iter().flatten());
                    extra.after.extend(collected.after);
                    extra.floats.extend(collected.floats);
                }
                "list" | "list-item" | "list-header" | "section" | "table" | "table-header-rows" | "table-rows" | "table-row-group" | "table-row" | "table-cell" => self.cell(child, spans, extra)?,
                name if DRAWINGS.contains(&name) => self.drawing(child, extra)?,
                _ => {}
            }
        }
        Ok(())
    }

    fn style(&mut self, text: &Props) -> (Style, bool) {
        let font = self.styles.font(text);
        let flag = |name: &str| text.get(name).is_some_and(|value| value != "none");
        let style = Style {
            bold: text.get("font-weight").is_some_and(|weight| weight == "bold" || weight.parse::<u16>().is_ok_and(|weight| weight >= 600)),
            italic: matches!(text.get("font-style").map(String::as_str), Some("italic" | "oblique")),
            code: false,
            strike: flag("text-line-through-style"),
            underline: flag("text-underline-style"),
            family: self.fonts.index(font.as_deref()),
            half_points: (font_size(text) * 2.).round() as u16,
            color: text.get("color").and_then(|value| hex_color(value.trim_start_matches('#'))),
        };
        (style, text.get("display").map(String::as_str) == Some("none"))
    }

    fn inline(&mut self, node: &Node, text: &Props, link: Option<&str>, out: &mut Collected) -> Result<(), ToolError> {
        let (style, hidden) = self.style(text);
        if hidden { return Ok(()); }
        for child in &node.children {
            let child = match child {
                Child::Text(value) => {
                    push_collapsed(out, value, style, link);
                    continue;
                }
                Child::Node(child) => child,
            };
            match child.name.as_str() {
                "span" => {
                    let props = self.styles.text(text, child.attr("style-name"));
                    self.inline(child, &props, link, out)?;
                }
                "a" if child.nodes().any(|node| DRAWINGS.contains(&node.name.as_str())) => for drawing in child.nodes() { self.drawing(drawing, out)?; },
                "a" => {
                    let target = child.attr("href").and_then(link_target);
                    self.inline(child, text, target.as_deref().or(link), out)?;
                }
                "s" => {
                    let count = child.attr("c").and_then(|value| value.parse::<usize>().ok()).unwrap_or(1).clamp(1, MAX_SPACES);
                    out.text(&" ".repeat(count), style, link);
                }
                "tab" => out.text(" ", style, link),
                "line-break" => out.text("\n", style, link),
                "bookmark" | "bookmark-start" => if out.bookmark.is_none() { out.bookmark = child.attr("name").map(str::to_string); },
                "page-number" if self.band => out.text(&PAGE_NUMBER.to_string(), style, None),
                "page-count" if self.band => out.text(&PAGE_COUNT.to_string(), style, None),
                "hidden-text" if child.attr("is-hidden") != Some("false") => {}
                name if SKIPPED.contains(&name) => {}
                name if DRAWINGS.contains(&name) => self.drawing(child, out)?,
                _ => self.inline(child, text, link, out)?,
            }
        }
        Ok(())
    }

    fn drawing(&mut self, node: &Node, out: &mut Collected) -> Result<(), ToolError> {
        match node.name.as_str() {
            "a" | "g" => for child in node.nodes() { self.drawing(child, out)?; },
            "frame" => self.frame(node, out)?,
            "custom-shape" | "rect" | "ellipse" | "circle" | "regular-polygon" => {
                let blocks = self.text_box(node)?;
                let graphic = self.styles.graphic(node.attr("style-name"));
                let (fill, line) = paint(&graphic, true);
                self.place(node, &graphic, None, blocks, fill, line, out);
            }
            _ => self.omitted.objects += 1,
        }
        Ok(())
    }

    fn frame(&mut self, node: &Node, out: &mut Collected) -> Result<(), ToolError> {
        let graphic = self.styles.graphic(node.attr("style-name"));
        if let Some(object) = node.nodes().find(|child| child.name == "object" || child.name == "object-ole") {
            match self.image(node)? {
                Some(index) => self.place(node, &graphic, Some(index), Vec::new(), None, None, out),
                None if self.chart(object) => self.omitted.charts += 1,
                None => self.omitted.objects += 1,
            }
        } else if let Some(content) = node.child("text-box") {
            let blocks = self.text_box(content)?;
            let (fill, line) = paint(&graphic, false);
            self.place(node, &graphic, None, blocks, fill, line, out);
        } else if node.child("image").is_some() {
            match self.image(node)? {
                Some(index) => self.place(node, &graphic, Some(index), Vec::new(), None, None, out),
                None => self.omitted.pictures += 1,
            }
        } else {
            self.omitted.objects += 1;
        }
        Ok(())
    }

    fn chart(&self, object: &Node) -> bool {
        let stored = object.attr("href").and_then(internal).and_then(|path| self.media.get(path.trim_end_matches('/')));
        stored.is_some_and(|kind| kind == CHART) || object.find("chart").is_some()
    }

    /// The first stored image of a frame that can be shown, at the frame's size.
    fn image(&mut self, frame: &Node) -> Result<Option<usize>, ToolError> {
        let size = frame_size(frame);
        for image in frame.nodes().filter(|node| node.name == "image") {
            let bytes = match (image.attr("href").and_then(internal), image.child("binary-data")) {
                (Some(path), _) => self.package.part(&path)?,
                (None, Some(data)) => BASE64.decode(data.text().split_whitespace().collect::<String>()).ok(),
                (None, None) => None,
            };
            if let Some(picture) = bytes.and_then(|bytes| Picture::from_bytes(&bytes, size).ok()) {
                self.pictures.push(picture);
                return Ok(Some(self.pictures.len() - 1));
            }
        }
        Ok(None)
    }

    fn text_box(&mut self, content: &Node) -> Result<Vec<Block>, ToolError> {
        let mut out = Out::default();
        for child in content.nodes() { self.block(child, &mut out)?; }
        Ok(out.finish())
    }

    #[allow(clippy::too_many_arguments)]
    fn place(&mut self, node: &Node, graphic: &Props, picture: Option<usize>, blocks: Vec<Block>, fill: Option<[u8; 3]>, line: Option<[u8; 3]>, out: &mut Collected) {
        let anchor = node.attr("anchor-type").unwrap_or("paragraph");
        if anchor == "as-char" {
            out.after.extend(picture.map(Block::Image));
            out.after.extend(blocks);
            return;
        }
        let (width, height) = frame_size(node).filter(|(width, height)| *width > 0. && *height > 0.).unwrap_or((DEFAULT_FLOAT_SIDE, DEFAULT_FLOAT_SIDE));
        let position = |relation: &str, place: &str, offset: &str| {
            let frame = match graphic.get(relation).map(String::as_str) {
                Some(relation) if relation.starts_with("page-content") => Frame::Margin,
                Some(relation) if relation.starts_with("page") => Frame::Page,
                Some(_) => Frame::Text,
                None if anchor == "page" => Frame::Page,
                None => Frame::Text,
            };
            let place = match graphic.get(place).map(String::as_str) {
                Some("left" | "top" | "inside") => Place::Start,
                Some("center" | "middle") => Place::Center,
                Some("right" | "bottom" | "outside") => Place::End,
                _ => Place::Offset(node.attr(offset).and_then(css_points).unwrap_or(0.)),
            };
            Position { frame, place }
        };
        let wrap = graphic.get("wrap").map(String::as_str) != Some("run-through");
        out.floats.push(Block::Float(Float {
            x: position("horizontal-rel", "horizontal-pos", "x"),
            y: position("vertical-rel", "vertical-pos", "y"),
            width,
            height,
            fill,
            line,
            picture,
            blocks,
            wrap,
        }));
    }
}

fn frame_size(node: &Node) -> Option<(f32, f32)> {
    Some((node.attr("width").and_then(css_points)?, node.attr("height").and_then(css_points)?))
}

/// Explicit colours from the drawing style, else a neutral stand-in when a shape asks for a fill or outline.
fn paint(graphic: &Props, shape: bool) -> (Option<[u8; 3]>, Option<[u8; 3]>) {
    let colour = |name: &str| graphic.get(name).and_then(|value| hex_color(value.trim().trim_start_matches('#')));
    let fill = match graphic.get("fill").map(String::as_str) {
        Some("none") => None,
        Some("solid") => colour("fill-color").or(colour("background-color")).or(Some(SHAPE_FILL)),
        _ => colour("background-color").or_else(|| shape.then(|| colour("fill-color").unwrap_or(SHAPE_FILL))),
    };
    let border = graphic.get("border").filter(|border| border.as_str() != "none").map(|border| {
        border.split_whitespace().find_map(|token| hex_color(token.trim_start_matches('#'))).unwrap_or([0, 0, 0])
    });
    let line = match graphic.get("stroke").map(String::as_str) {
        Some("none") => border,
        Some(_) => colour("stroke-color").or(Some(SHAPE_LINE)),
        None => border.or_else(|| shape.then(|| colour("stroke-color").unwrap_or(SHAPE_LINE))),
    };
    (fill, line)
}

/// Runs of white space in document text show as one space, and none at the start of a paragraph.
fn push_collapsed(out: &mut Collected, value: &str, style: Style, link: Option<&str>) {
    let mut space = out.segments.last().and_then(|spans| spans.last()).is_none_or(|span| span.text.ends_with([' ', '\n']));
    let mut text = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(character, ' ' | '\t' | '\n' | '\r') {
            if !space { text.push(' '); }
            space = true;
        } else {
            text.push(character);
            space = false;
        }
    }
    out.text(&text, style, link);
}

fn formatted(spans: Vec<Span>, style: &ParaStyle) -> Paragraph {
    let length = |name: &str| style.para.get(name).or(style.para.get("margin").filter(|_| name.starts_with("margin"))).and_then(|value| css_points(value)).unwrap_or(0.).max(0.);
    let size = font_size(&style.text);
    let leading = match (style.para.get("line-height"), style.para.get("line-height-at-least")) {
        (Some(height), _) if height.ends_with('%') => height.trim_end_matches('%').trim().parse::<f32>().map(|percent| (1.2 * percent / 100.).clamp(0.6, 6.)).unwrap_or(1.2),
        (Some(height), _) => css_points(height).map(|points| (points / size).clamp(0.3, 6.)).unwrap_or(1.2),
        (None, Some(least)) => css_points(least).map(|points| (points / size).clamp(1.2, 6.)).unwrap_or(1.2),
        (None, None) => 1.2,
    };
    Paragraph {
        spans,
        align: match style.para.get("text-align").map(String::as_str) {
            Some("center") => Align::Center,
            Some("end" | "right") => Align::Right,
            _ => Align::Left,
        },
        indent: length("margin-left"),
        right_indent: length("margin-right"),
        before: length("margin-top"),
        after: length("margin-bottom"),
        size,
        leading,
    }
}

#[cfg(test)]
pub mod fixtures {
    use std::io::{Cursor, Write};

    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    pub const NS: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:loext="urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0" office:version="1.3""#;

    /// Builds an ODT package from automatic styles, a body and extra parts; `entries` add `(path, media type)` lines to the manifest.
    pub fn odt(automatic: &str, body: &str, parts: &[(&str, &[u8])], entries: &[(&str, &str)]) -> Vec<u8> {
        let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
        archive.start_file("mimetype", SimpleFileOptions::default().compression_method(CompressionMethod::Stored)).unwrap();
        archive.write_all(b"application/vnd.oasis.opendocument.text").unwrap();
        let mut add = |name: &str, data: &[u8]| {
            archive.start_file(name, SimpleFileOptions::default()).unwrap();
            archive.write_all(data).unwrap();
        };
        let lines = entries.iter().map(|(path, kind)| format!(r#"<manifest:file-entry manifest:full-path="{path}" manifest:media-type="{kind}"/>"#)).collect::<String>();
        add("META-INF/manifest.xml", format!(r#"<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.3"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.text"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>{lines}</manifest:manifest>"#).as_bytes());
        add("content.xml", format!(r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content {NS}><office:automatic-styles>{automatic}</office:automatic-styles><office:body><office:text>{body}</office:text></office:body></office:document-content>"#).as_bytes());
        for (name, data) in parts { add(name, data); }
        archive.finish().unwrap().into_inner()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::contracts::{ErrorKind, LimitKind};

    fn exceeded() -> ToolError { ToolError::limit_exceeded(LimitKind::Decompression, "over") }
    fn locked() -> ToolError { ToolError::invalid_input("locked") }

    #[test]
    fn the_expansion_budget_covers_every_part_read() {
        let body = format!("<text:p>{}</text:p>", "x".repeat(4_000));
        let bytes = fixtures::odt("", &body, &[], &[]);
        assert!(parse(&bytes, 1_000_000, exceeded(), locked()).is_ok());
        let error = parse(&bytes, 3_000, exceeded(), locked()).err().unwrap();
        assert!(matches!(error.kind, ErrorKind::LimitExceeded { .. }), "{error:?}");
    }

    #[test]
    fn unbalanced_or_foreign_content_is_damaged() {
        assert_eq!(parse(&fixtures::odt("", "<text:p>", &[], &[]), 1_000_000, exceeded(), locked()).err().unwrap().message, DAMAGED);
        assert_eq!(parse(b"PK\x03\x04 not a package", 1_000_000, exceeded(), locked()).err().unwrap().message, DAMAGED);
        let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        archive.start_file("content.xml", zip::write::SimpleFileOptions::default()).unwrap();
        std::io::Write::write_all(&mut archive, format!(r#"<office:document-content {}><office:body><office:spreadsheet/></office:body></office:document-content>"#, fixtures::NS).as_bytes()).unwrap();
        assert_eq!(parse(&archive.finish().unwrap().into_inner(), 1_000_000, exceeded(), locked()).err().unwrap().message, DAMAGED);
    }

    #[test]
    fn an_encrypted_manifest_is_locked_before_any_content_is_read() {
        let manifest = br#"<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0"><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"><manifest:encryption-data manifest:checksum-type="SHA1/1K" manifest:checksum="AA=="><manifest:algorithm manifest:algorithm-name="Blowfish CFB" manifest:initialisation-vector="AA=="/></manifest:encryption-data></manifest:file-entry></manifest:manifest>"#;
        let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, data) in [("META-INF/manifest.xml", &manifest[..]), ("content.xml", b"\x00\x01 encrypted bytes")] {
            archive.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            std::io::Write::write_all(&mut archive, data).unwrap();
        }
        assert_eq!(parse(&archive.finish().unwrap().into_inner(), 1_000_000, exceeded(), locked()).err().unwrap().message, "locked");
    }

    #[test]
    fn links_stay_inside_the_package_or_keep_safe_schemes() {
        assert_eq!(internal("./Pictures/a.png"), Some("Pictures/a.png".into()));
        assert_eq!(internal("../outside.png"), None);
        assert_eq!(internal("Pictures/../../outside.png"), None);
        assert_eq!(internal("file:///etc/hosts"), None);
        assert_eq!(internal("/etc/hosts"), None);
        assert_eq!(link_target("#summary"), Some("#summary".into()));
        assert_eq!(link_target("https://example.com"), Some("https://example.com".into()));
        assert_eq!(link_target("file:///etc/hosts"), None);
        assert_eq!(link_target("../sibling.odt"), None);
    }

    #[test]
    fn white_space_collapses_and_relative_font_sizes_scale() {
        let mut out = Collected::new();
        push_collapsed(&mut out, "  Two\n   words ", Style::default(), None);
        push_collapsed(&mut out, " here", Style::default(), None);
        assert_eq!(plain_text(&out.segments[0]), "Two words here");
        let mut text = Props::from([("font-size".to_string(), "10pt".to_string())]);
        merge_text(&mut text, &Props::from([("font-size".to_string(), "150%".to_string())]));
        assert_eq!(font_size(&text), 15.);
    }
}
