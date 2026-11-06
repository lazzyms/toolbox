//! HTML pages on the shared block layout. Scripts never run (the parser works with scripting off
//! and script elements are skipped), and nothing is fetched: remote stylesheets, fonts, images and
//! embedded pages are counted as left out. Local stylesheets and images resolve against the page's folder.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use base64::Engine;
use dom_query::{Document, Matcher, NodeRef};

use crate::kit::pdf::layout::{plain_text, Align, Block, Paragraph, Picture, Span, Style};
use crate::kit::pdf::markdown::{fs_key, local_image, slug};

/// Browsers start text at 16px, which is 12pt.
pub const BODY_SIZE: f32 = 12.;
const MAX_IMPORT_DEPTH: usize = 4;
const SKIPPED: &[&str] = &["head", "script", "style", "template", "title", "meta", "link", "base", "noembed", "input", "select", "textarea", "svg", "math", "canvas", "map", "area", "param", "track", "source"];
const EMBEDS: &[&str] = &["iframe", "frame", "embed", "object", "video", "audio"];
const BLOCKS: &[&str] = &["html", "body", "p", "div", "section", "article", "header", "footer", "main", "nav", "aside", "figure", "figcaption", "address", "center", "form", "fieldset", "legend", "details", "summary", "dl", "dt", "dd", "li", "caption", "hgroup", "search", "dialog", "frameset", "noscript"];

/// Remote resources that were left out, each counted once however often the page uses it.
#[derive(Debug, Default)]
pub struct LeftOut { pub images: BTreeSet<String>, pub stylesheets: BTreeSet<String>, pub fonts: BTreeSet<String>, pub embeds: BTreeSet<String> }

#[derive(Default)]
pub struct Parsed {
    pub blocks: Vec<Block>,
    pub title: Option<String>,
    pub pictures: Vec<Picture>,
    pub families: Vec<String>,
    pub background: Option<[u8; 3]>,
    pub remote: LeftOut,
    pub unreadable_images: usize,
    pub unreadable_stylesheets: usize,
}

struct Declaration { property: String, value: String, important: bool }

struct Rule { matcher: Matcher, specificity: (u32, u32, u32), order: usize, declarations: std::rc::Rc<Vec<Declaration>> }

#[derive(Clone, Default)]
struct Computed { bold: bool, italic: bool, underline: bool, strike: bool, mono: bool, color: Option<[u8; 3]>, family: Option<String>, size: Option<f32>, align: Align, link: Option<String> }

struct Out { blocks: Vec<Block>, spans: Vec<Span>, align: Align, inline_only: bool }

impl Out {
    fn new(align: Align, inline_only: bool) -> Self { Self { blocks: Vec::new(), spans: Vec::new(), align, inline_only } }

    fn push(&mut self, text: &str, style: Style, link: Option<String>) {
        match self.spans.last_mut() {
            Some(last) if last.style == style && last.link == link => last.text.push_str(text),
            _ => self.spans.push(Span { text: text.to_string(), style, link }),
        }
    }

    fn flush(&mut self) {
        let spans = std::mem::take(&mut self.spans);
        if !spans.iter().any(|span| !span.text.trim().is_empty()) { return; }
        self.blocks.push(match self.align {
            Align::Left => Block::Paragraph(spans),
            align => Block::Formatted(Paragraph { spans, align, indent: 0., right_indent: 0., before: 0., after: BODY_SIZE * 0.6, size: BODY_SIZE, leading: 1.45 }),
        });
    }

    /// Text gathered for a heading or table cell; nested paragraphs join with line breaks.
    fn into_spans(mut self) -> Vec<Span> {
        self.flush();
        let mut spans = Vec::new();
        for block in self.blocks {
            let part = match block { Block::Paragraph(part) => part, Block::Formatted(paragraph) => paragraph.spans, _ => continue };
            if let Some(last) = spans.last() { spans.push(Span { text: "\n".into(), ..Clone::clone(last) }); }
            spans.extend(part);
        }
        spans
    }
}

struct Walker<'a> {
    base: Option<&'a Path>,
    rules: Vec<Rule>,
    parsed: Parsed,
    loaded: BTreeMap<String, usize>,
    anchors: HashSet<String>,
    html_background: Option<[u8; 3]>,
    body_background: Option<[u8; 3]>,
}

/// Reads `markup`; `base` is the folder relative stylesheets and images resolve against, or `None`
/// for pasted markup, where only absolute `file:` addresses are local.
pub fn parse(markup: &str, base: Option<&Path>) -> Parsed {
    let document = Document::from(markup);
    let mut walker = Walker { base, rules: Vec::new(), parsed: Parsed::default(), loaded: BTreeMap::new(), anchors: HashSet::new(), html_background: None, body_background: None };
    walker.stylesheets(&document);
    let mut out = Out::new(Align::Left, false);
    walker.children(&document.root(), &Computed::default(), &mut out);
    out.flush();
    let mut parsed = walker.parsed;
    parsed.blocks = out.blocks;
    parsed.background = walker.html_background.or(walker.body_background);
    parsed.title = document.select("head title").nodes().first().map(|title| collapse(&title.text()).trim().to_string()).filter(|title| !title.is_empty())
        .or_else(|| parsed.blocks.iter().find_map(|block| match block { Block::Heading { level: 1, spans, .. } => Some(plain_text(spans)), _ => None }));
    parsed
}

fn collapse(text: &str) -> String {
    let mut collapsed = String::with_capacity(text.len());
    let mut space = false;
    for character in text.chars() {
        if matches!(character, ' ' | '\t' | '\n' | '\r' | '\u{c}') {
            if !space { collapsed.push(' '); }
            space = true;
        } else {
            collapsed.push(character);
            space = false;
        }
    }
    collapsed
}

fn is_remote(url: &str) -> bool {
    let url = url.trim().to_ascii_lowercase();
    ["http://", "https://", "ftp://", "//"].iter().any(|prefix| url.starts_with(prefix))
}

/// Addresses inside `url(...)` in a CSS value or `@import` prelude.
fn urls(value: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = value;
    while let Some(start) = rest.to_ascii_lowercase().find("url(") {
        let after = &rest[start + 4..];
        let Some(end) = after.find(')') else { break };
        found.push(after[..end].trim().trim_matches(|c| c == '"' || c == '\'').to_string());
        rest = &after[end + 1..];
    }
    found
}

fn media_applies(query: &str) -> bool {
    let query = query.trim().to_ascii_lowercase();
    query.is_empty() || (!query.starts_with("not ") && (query.contains("print") || query.contains("all")))
}

/// The first `wanted` character outside quotes and parentheses.
fn find_top(text: &str, wanted: &[char]) -> Option<(usize, char)> {
    let (mut quote, mut depth) = (None, 0usize);
    for (index, character) in text.char_indices() {
        match (quote, character) {
            (Some(open), _) if character == open => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(character),
            (None, '(') => depth += 1,
            (None, ')') => depth = depth.saturating_sub(1),
            (None, _) if depth == 0 && wanted.contains(&character) => return Some((index, character)),
            _ => {}
        }
    }
    None
}

/// The index of the brace closing the one at `open`, or the end of the text when it never closes.
fn closing_brace(text: &str, open: usize) -> usize {
    let mut depth = 0usize;
    let mut quote = None;
    for (index, character) in text[open..].char_indices() {
        match (quote, character) {
            (Some(q), _) if character == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(character),
            (None, '{') => depth += 1,
            (None, '}') => {
                depth -= 1;
                if depth == 0 { return open + index; }
            }
            _ => {}
        }
    }
    text.len()
}

fn strip_comments(css: &str) -> String {
    let mut stripped = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        stripped.push_str(&rest[..start]);
        rest = rest[start + 2..].find("*/").map(|end| &rest[start + 2 + end + 2..]).unwrap_or("");
    }
    stripped.push_str(rest);
    stripped
}

fn declarations(body: &str) -> Vec<Declaration> {
    let mut parsed = Vec::new();
    let mut rest = body;
    loop {
        let (item, next) = match find_top(rest, &[';']) { Some((index, _)) => (&rest[..index], &rest[index + 1..]), None => (rest, "") };
        if let Some((property, value)) = item.split_once(':') {
            let property = property.trim().to_ascii_lowercase();
            let mut value = value.trim().to_string();
            let lower = value.to_ascii_lowercase();
            let important = lower.trim_end().ends_with("important") && lower.contains('!');
            if important { value.truncate(value.rfind('!').unwrap_or(value.len())); }
            let value = value.trim().to_string();
            if !property.is_empty() && !property.starts_with("--") && !value.is_empty() && !value.contains("var(") {
                parsed.push(Declaration { property, value, important });
            }
        }
        if next.is_empty() { break; }
        rest = next;
    }
    parsed
}

/// Ids, then classes, attributes and pseudo-classes, then element names.
fn specificity(selector: &str) -> (u32, u32, u32) {
    let (mut ids, mut classes, mut types) = (0, 0, 0);
    let characters = selector.chars().collect::<Vec<_>>();
    let mut index = 0;
    let mut compound_start = true;
    let ident = |index: &mut usize| while *index < characters.len() && (characters[*index].is_alphanumeric() || matches!(characters[*index], '-' | '_' | '\\')) { *index += 1; };
    while index < characters.len() {
        match characters[index] {
            '#' => { ids += 1; index += 1; ident(&mut index); compound_start = false; }
            '.' => { classes += 1; index += 1; ident(&mut index); compound_start = false; }
            '[' => {
                classes += 1;
                while index < characters.len() && characters[index] != ']' { index += 1; }
                index += 1;
                compound_start = false;
            }
            ':' => {
                index += 1;
                ident(&mut index);
                classes += 1;
                if characters.get(index) == Some(&'(') {
                    let mut depth = 0;
                    while index < characters.len() {
                        if characters[index] == '(' { depth += 1; }
                        if characters[index] == ')' { depth -= 1; if depth == 0 { index += 1; break; } }
                        index += 1;
                    }
                }
                compound_start = false;
            }
            '*' => { index += 1; compound_start = false; }
            ' ' | '>' | '+' | '~' => { index += 1; compound_start = true; }
            character if compound_start && (character.is_alphabetic() || character == '_') => { types += 1; ident(&mut index); compound_start = false; }
            _ => index += 1,
        }
    }
    (ids, classes, types)
}

fn hex(value: &str) -> Option<[u8; 3]> {
    let digits = value.chars().map(|character| character.to_digit(16).map(|digit| digit as u8)).collect::<Option<Vec<_>>>()?;
    match digits.len() {
        3 | 4 => Some([digits[0] * 17, digits[1] * 17, digits[2] * 17]),
        6 | 8 => Some([digits[0] * 16 + digits[1], digits[2] * 16 + digits[3], digits[4] * 16 + digits[5]]),
        _ => None,
    }
}

/// `Some(None)` is a transparent colour; `None` is a value this subset doesn't read.
fn color(value: &str) -> Option<Option<[u8; 3]>> {
    let value = value.trim().to_ascii_lowercase();
    if let Some(digits) = value.strip_prefix('#') { return hex(digits).map(Some); }
    if let Some(arguments) = value.strip_prefix("rgba(").or_else(|| value.strip_prefix("rgb(")).and_then(|rest| rest.strip_suffix(')')) {
        let parts = arguments.split(|c: char| c == ',' || c == '/' || c.is_whitespace()).filter(|part| !part.is_empty()).collect::<Vec<_>>();
        if parts.len() < 3 { return None; }
        if parts.get(3).is_some_and(|alpha| alpha.trim_end_matches('%').parse::<f32>().is_ok_and(|alpha| alpha == 0.)) { return Some(None); }
        let channel = |part: &str| match part.strip_suffix('%') {
            Some(percent) => percent.parse::<f32>().ok().map(|value| (value * 2.55).round().clamp(0., 255.) as u8),
            None => part.parse::<f32>().ok().map(|value| value.round().clamp(0., 255.) as u8),
        };
        return Some(Some([channel(parts[0])?, channel(parts[1])?, channel(parts[2])?]));
    }
    let named = match value.as_str() {
        "transparent" => return Some(None),
        "black" => "000000", "white" => "ffffff", "red" => "ff0000", "green" => "008000", "blue" => "0000ff",
        "gray" | "grey" => "808080", "silver" => "c0c0c0", "maroon" => "800000", "purple" => "800080", "fuchsia" | "magenta" => "ff00ff",
        "lime" => "00ff00", "olive" => "808000", "yellow" => "ffff00", "navy" => "000080", "teal" => "008080", "aqua" | "cyan" => "00ffff",
        "orange" => "ffa500", "brown" => "a52a2a", "pink" => "ffc0cb", "gold" => "ffd700", "darkgray" | "darkgrey" => "a9a9a9",
        "lightgray" | "lightgrey" => "d3d3d3", "dimgray" | "dimgrey" => "696969", "darkred" => "8b0000", "darkgreen" => "006400",
        "darkblue" => "00008b", "crimson" => "dc143c", "tomato" => "ff6347", "beige" => "f5f5dc", "ivory" => "fffff0", "whitesmoke" => "f5f5f5",
        _ => return None,
    };
    hex(named).map(Some)
}

fn font_size(value: &str, parent: f32) -> Option<f32> {
    let value = value.trim().to_ascii_lowercase();
    let number = |suffix: &str| value.strip_suffix(suffix).and_then(|number| number.trim().parse::<f32>().ok());
    let size = match value.as_str() {
        "xx-small" => 7., "x-small" => 7.5, "small" => 10., "medium" => 12., "large" => 13.5, "x-large" => 18., "xx-large" => 24., "xxx-large" => 36.,
        "smaller" => parent / 1.2, "larger" => parent * 1.2,
        _ => number("px").map(|px| px * 0.75).or_else(|| number("pt")).or_else(|| number("rem").map(|rem| rem * BODY_SIZE))
            .or_else(|| number("em").map(|em| em * parent)).or_else(|| number("%").map(|percent| percent / 100. * parent))?,
    };
    Some(size.clamp(4., 200.))
}

impl Computed {
    fn style(&self, family: u16) -> Style {
        Style { bold: self.bold, italic: self.italic, code: self.mono, strike: self.strike, underline: self.underline, family, half_points: self.size.map(|size| (size * 2.).round() as u16).unwrap_or(0), color: self.color }
    }

    fn families(&mut self, value: &str) {
        let names = value.split(',').map(|name| name.trim().trim_matches(|c| c == '"' || c == '\'').to_string()).filter(|name| !name.is_empty()).collect::<Vec<_>>();
        let generic = |name: &str| match name.to_ascii_lowercase().as_str() {
            "monospace" | "ui-monospace" | "courier" | "courier new" | "consolas" | "menlo" | "monaco" => Some(None),
            "serif" | "ui-serif" => Some(Some("Times New Roman")),
            "sans-serif" | "ui-sans-serif" | "system-ui" | "-apple-system" | "blinkmacsystemfont" => Some(Some("Arial")),
            _ => None,
        };
        let Some(first) = names.first() else { return };
        match generic(first) {
            Some(None) => { self.mono = true; self.family = None; }
            Some(Some(family)) => { self.mono = false; self.family = Some(family.to_string()); }
            None => { self.mono = names.iter().any(|name| generic(name) == Some(None)) && names.len() == 1; self.family = Some(first.clone()); }
        }
    }

    fn font(&mut self, value: &str) {
        let parent = self.size.unwrap_or(BODY_SIZE);
        let tokens = value.split_whitespace().collect::<Vec<_>>();
        let size_of = |token: &str| font_size(token.split('/').next().unwrap_or(token), parent);
        let Some(position) = tokens.iter().position(|token| size_of(token).is_some()) else { return };
        let before = tokens[..position].iter().map(|token| token.to_ascii_lowercase()).collect::<Vec<_>>();
        self.bold = before.iter().any(|token| token == "bold" || token == "bolder" || token.parse::<u16>().is_ok_and(|weight| weight >= 600));
        self.italic = before.iter().any(|token| token == "italic" || token == "oblique");
        self.size = size_of(tokens[position]);
        let families = tokens[position + 1..].join(" ");
        if !families.is_empty() { self.families(&families); }
    }
}

impl Walker<'_> {
    fn stylesheets(&mut self, document: &Document) {
        for node in document.select("style, link").nodes() {
            if !media_applies(&node.attr("media").map(|media| media.to_string()).unwrap_or_default()) { continue; }
            if node.node_name().is_some_and(|name| name.eq_ignore_ascii_case("style")) {
                self.stylesheet(&node.text(), self.base.map(Path::to_path_buf).as_deref(), 0);
                continue;
            }
            let rel = node.attr("rel").map(|rel| rel.to_ascii_lowercase()).unwrap_or_default();
            if !rel.split_whitespace().any(|token| token == "stylesheet") || rel.split_whitespace().any(|token| token == "alternate") { continue; }
            if let Some(href) = node.attr("href") { self.linked(&href, self.base.map(Path::to_path_buf).as_deref(), 0); }
        }
    }

    fn linked(&mut self, href: &str, base: Option<&Path>, depth: usize) {
        if is_remote(href) { self.parsed.remote.stylesheets.insert(href.trim().to_string()); return; }
        let file = local_image(href, base.unwrap_or(Path::new(""))).filter(|_| base.is_some() || href.trim().to_ascii_lowercase().starts_with("file:"))
            .and_then(|candidates| candidates.into_iter().find(|path| path.is_file()));
        match file.and_then(|path| std::fs::read(&path).ok().map(|bytes| (path, bytes))) {
            Some((path, bytes)) if depth < MAX_IMPORT_DEPTH => self.stylesheet(&String::from_utf8_lossy(&bytes), path.parent(), depth + 1),
            Some(_) => {}
            None => self.parsed.unreadable_stylesheets += 1,
        }
    }

    fn stylesheet(&mut self, css: &str, base: Option<&Path>, depth: usize) {
        let css = strip_comments(css);
        let mut rest = css.as_str();
        loop {
            rest = rest.trim_start();
            if rest.is_empty() { break; }
            if let Some(at) = rest.strip_prefix('@') {
                let name_end = at.find(|c: char| !(c.is_alphanumeric() || c == '-')).unwrap_or(at.len());
                let name = at[..name_end].to_ascii_lowercase();
                match find_top(rest, &[';', '{']) {
                    Some((end, ';')) => {
                        if name == "import" { self.import(&rest[1 + name_end..end], base, depth); }
                        rest = &rest[end + 1..];
                    }
                    Some((open, _)) => {
                        let close = closing_brace(rest, open);
                        let prelude = &rest[1 + name_end..open];
                        let body = &rest[open + 1..close.min(rest.len())];
                        match name.as_str() {
                            "media" if media_applies(prelude) => self.stylesheet(body, base, depth),
                            "supports" | "layer" | "document" | "container" => self.stylesheet(body, base, depth),
                            "font-face" => {
                                if let Some(remote) = declarations(body).iter().filter(|declaration| declaration.property == "src").flat_map(|declaration| urls(&declaration.value)).find(|url| is_remote(url)) {
                                    self.parsed.remote.fonts.insert(remote);
                                }
                            }
                            _ => {}
                        }
                        rest = rest.get(close + 1..).unwrap_or("");
                    }
                    None => break,
                }
                continue;
            }
            let Some((open, _)) = find_top(rest, &['{']) else { break };
            let close = closing_brace(rest, open);
            let declarations = std::rc::Rc::new(declarations(&rest[open + 1..close.min(rest.len())]));
            for selector in rest[..open].split(',').map(str::trim).filter(|selector| !selector.is_empty()) {
                if let Ok(matcher) = Matcher::new(selector) {
                    self.rules.push(Rule { matcher, specificity: specificity(selector), order: self.rules.len(), declarations: declarations.clone() });
                }
            }
            rest = rest.get(close + 1..).unwrap_or("");
        }
    }

    fn import(&mut self, prelude: &str, base: Option<&Path>, depth: usize) {
        let prelude = prelude.trim();
        let (target, media) = match urls(prelude).into_iter().next() {
            Some(url) => (url, prelude.get(prelude.find(')').map(|end| end + 1).unwrap_or(prelude.len())..).unwrap_or("")),
            None => {
                let quote = prelude.chars().next().filter(|c| *c == '"' || *c == '\'');
                let Some(quote) = quote else { return };
                let end = prelude[1..].find(quote).map(|end| end + 1).unwrap_or(prelude.len());
                (prelude[1..end].to_string(), prelude.get(end + 1..).unwrap_or(""))
            }
        };
        if media_applies(media) { self.linked(&target, base, depth); }
    }

    fn compute(&mut self, node: &NodeRef, name: &str, parent: &Computed) -> Option<Computed> {
        let mut style = parent.clone();
        let parent_size = parent.size.unwrap_or(BODY_SIZE);
        match name {
            "b" | "strong" | "th" => style.bold = true,
            "i" | "em" | "cite" | "var" | "dfn" | "address" => style.italic = true,
            "u" | "ins" => style.underline = true,
            "s" | "strike" | "del" => style.strike = true,
            "code" | "kbd" | "samp" | "tt" => style.mono = true,
            "small" => style.size = Some(parent_size * 0.83),
            "big" => style.size = Some(parent_size * 1.2),
            "center" => style.align = Align::Center,
            "a" => style.link = node.attr("href").map(|href| href.trim().to_string()).filter(|href| !href.is_empty() && !href.to_ascii_lowercase().starts_with("javascript:")),
            "font" => {
                if let Some(Some(rgb)) = node.attr("color").and_then(|value| color(&value)) { style.color = Some(rgb); }
                if let Some(face) = node.attr("face") { style.families(&face); }
            }
            _ => {}
        }
        if name == "th" { style.align = Align::Center; }
        if let Some(align) = node.attr("align") { style.align = alignment(&align).unwrap_or(style.align); }
        if name == "body" {
            if let Some(Some(rgb)) = node.attr("bgcolor").and_then(|value| color(&value)) { self.body_background = Some(rgb); }
        }

        let mut winning = Vec::new();
        for rule in &self.rules {
            if node.is_match(&rule.matcher) {
                for declaration in rule.declarations.iter() { winning.push(((declaration.important, false, rule.specificity, rule.order), declaration)); }
            }
        }
        let inline = node.attr("style").map(|style| declarations(&style)).unwrap_or_default();
        for declaration in &inline { winning.push(((declaration.important, true, (0, 0, 0), usize::MAX), declaration)); }
        winning.sort_by_key(|(key, _)| *key);
        for (_, declaration) in winning {
            let value = declaration.value.as_str();
            let lower = value.to_ascii_lowercase();
            if lower == "inherit" || lower == "unset" { continue; }
            match declaration.property.as_str() {
                "display" if lower == "none" => return None,
                "color" => if let Some(rgb) = color(value) { style.color = rgb; },
                "font-weight" => style.bold = matches!(lower.as_str(), "bold" | "bolder") || lower.parse::<u16>().is_ok_and(|weight| weight >= 600),
                "font-style" => style.italic = lower.starts_with("italic") || lower.starts_with("oblique"),
                "text-decoration" | "text-decoration-line" => {
                    style.underline = lower.contains("underline");
                    style.strike = lower.contains("line-through");
                }
                "text-align" => style.align = alignment(&lower).unwrap_or(style.align),
                "font-family" => style.families(value),
                "font-size" => if let Some(size) = font_size(&lower, parent_size) { style.size = Some(size); },
                "font" => style.font(value),
                property => {
                    if property == "background" || property == "background-color" {
                        let background = if property == "background-color" { color(value) } else { lower.split_whitespace().find_map(color) };
                        if let Some(background) = background {
                            match name { "html" => self.html_background = background, "body" => self.body_background = background, _ => {} }
                        }
                    }
                    if property.starts_with("background") || property.starts_with("list-style") || property.starts_with("border-image") || property == "content" {
                        for url in urls(value).into_iter().filter(|url| is_remote(url)) { self.parsed.remote.images.insert(url); }
                    }
                }
            }
        }
        Some(style)
    }

    fn family(&mut self, style: &Computed) -> u16 {
        let Some(family) = &style.family else { return 0 };
        let index = match self.parsed.families.iter().position(|known| known == family) {
            Some(index) => index,
            None => { self.parsed.families.push(family.clone()); self.parsed.families.len() - 1 }
        };
        index as u16 + 1
    }

    fn text(&mut self, text: &str, style: &Computed, out: &mut Out) {
        let mut text = collapse(text);
        if out.spans.last().is_none_or(|span| span.text.ends_with([' ', '\n'])) { text = text.trim_start().to_string(); }
        if text.is_empty() { return; }
        let family = self.family(style);
        out.push(&text, style.style(family), style.link.clone());
    }

    fn children(&mut self, node: &NodeRef, style: &Computed, out: &mut Out) {
        for child in node.children() {
            if child.is_text() { self.text(&child.text(), style, out); }
            else if child.is_element() { self.element(&child, style, out); }
        }
    }

    fn inline(&mut self, node: &NodeRef, style: &Computed) -> Vec<Span> {
        let mut out = Out::new(style.align, true);
        self.children(node, style, &mut out);
        out.into_spans()
    }

    fn element(&mut self, node: &NodeRef, parent: &Computed, out: &mut Out) {
        let name = node.node_name().map(|name| name.to_ascii_lowercase()).unwrap_or_default();
        if SKIPPED.contains(&name.as_str()) || node.has_attr("hidden") { return; }
        if EMBEDS.contains(&name.as_str()) {
            let sources = ["src", "data"].iter().filter_map(|attribute| node.attr(attribute)).map(|value| value.to_string())
                .chain(node.element_children().iter().filter(|child| child.node_name().is_some_and(|child| child.eq_ignore_ascii_case("source"))).filter_map(|child| child.attr("src")).map(|value| value.to_string()))
                .filter(|source| is_remote(source)).collect::<Vec<_>>();
            if let Some(first) = sources.into_iter().next() { self.parsed.remote.embeds.insert(first); }
            if let Some(poster) = node.attr("poster").filter(|poster| is_remote(poster)) { self.parsed.remote.images.insert(poster.to_string()); }
            return;
        }
        let Some(style) = self.compute(node, &name, parent) else { return };
        match name.as_str() {
            "br" => out.push("\n", style.style(0), style.link.clone()),
            "img" => self.image(node, &style, out),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" if !out.inline_only => {
                out.flush();
                let spans = self.inline(node, &style);
                if spans.iter().all(|span| span.text.trim().is_empty()) { return; }
                let level = name.as_bytes()[1] - b'0';
                let preferred = node.attr("id").map(|id| id.to_string()).filter(|id| !id.is_empty());
                let anchor = self.anchor(preferred, &spans);
                out.blocks.push(Block::Heading { level, spans, anchor });
            }
            "ul" | "ol" | "menu" | "dir" if !out.inline_only => {
                out.flush();
                let start = (name == "ol").then(|| node.attr("start").and_then(|start| start.trim().parse::<u64>().ok()).unwrap_or(1));
                let mut items: Vec<Vec<Block>> = Vec::new();
                for child in node.children() {
                    let is_item = child.node_name().is_some_and(|child| child.eq_ignore_ascii_case("li"));
                    let mut item = Out::new(style.align, false);
                    if child.is_element() && is_item {
                        let Some(item_style) = self.compute(&child, "li", &style) else { continue };
                        item.align = item_style.align;
                        self.children(&child, &item_style, &mut item);
                    } else if child.is_element() { self.element(&child, &style, &mut item); }
                    else if child.is_text() { self.text(&child.text(), &style, &mut item); }
                    item.flush();
                    match (is_item, items.last_mut()) {
                        (false, Some(last)) => last.extend(item.blocks),
                        _ if is_item || !item.blocks.is_empty() => items.push(item.blocks),
                        _ => {}
                    }
                }
                if !items.is_empty() { out.blocks.push(Block::List { start, items }); }
            }
            "blockquote" if !out.inline_only => {
                out.flush();
                let mut inner = Out::new(style.align, false);
                self.children(node, &style, &mut inner);
                inner.flush();
                if !inner.blocks.is_empty() { out.blocks.push(Block::Quote(inner.blocks)); }
            }
            "pre" | "listing" | "xmp" | "plaintext" if !out.inline_only => {
                out.flush();
                let text = node.text().to_string();
                let text = text.strip_prefix("\r\n").or_else(|| text.strip_prefix('\n')).unwrap_or(&text);
                out.blocks.push(Block::Code(text.to_string()));
            }
            "hr" if !out.inline_only => { out.flush(); out.blocks.push(Block::Rule); }
            "table" if !out.inline_only => { out.flush(); self.table(node, &style, out); }
            "q" => {
                out.push("\u{201c}", style.style(self.family(&style)), style.link.clone());
                self.children(node, &style, out);
                out.push("\u{201d}", style.style(self.family(&style)), style.link.clone());
            }
            _ if BLOCKS.contains(&name.as_str()) || ["h1", "h2", "h3", "h4", "h5", "h6", "ul", "ol", "menu", "dir", "blockquote", "pre", "listing", "xmp", "plaintext", "table", "tr"].contains(&name.as_str()) => {
                if out.inline_only {
                    if !out.spans.is_empty() { out.push("\n", style.style(0), None); }
                    self.children(node, &style, out);
                    return;
                }
                out.flush();
                let align = std::mem::replace(&mut out.align, style.align);
                self.children(node, &style, out);
                out.flush();
                out.align = align;
            }
            _ => self.children(node, &style, out),
        }
    }

    fn anchor(&mut self, preferred: Option<String>, spans: &[Span]) -> String {
        let base = preferred.unwrap_or_else(|| slug(&plain_text(spans)));
        let base = if base.is_empty() { "section".to_string() } else { base };
        let mut anchor = base.clone();
        let mut counter = 1;
        while !self.anchors.insert(anchor.clone()) {
            anchor = format!("{base}-{counter}");
            counter += 1;
        }
        anchor
    }

    fn image(&mut self, node: &NodeRef, style: &Computed, out: &mut Out) {
        let alt = node.attr("alt").map(|alt| collapse(&alt).trim().to_string()).unwrap_or_default();
        let source = node.attr("src").map(|source| source.trim().to_string()).unwrap_or_default();
        let fallback = |walker: &mut Self, out: &mut Out| if !alt.is_empty() { walker.text(&alt, style, out); };
        if source.is_empty() { return fallback(self, out); }
        if is_remote(&source) {
            self.parsed.remote.images.insert(source);
            return fallback(self, out);
        }
        if out.inline_only { return fallback(self, out); }
        let dimension = |attribute: &str| node.attr(attribute).and_then(|value| value.trim().trim_end_matches("px").parse::<f32>().ok()).filter(|value| *value > 0.);
        let size = dimension("width").zip(dimension("height")).map(|(width, height)| (width * 0.75, height * 0.75));
        let loaded = match source.get(..5).is_some_and(|scheme| scheme.eq_ignore_ascii_case("data:")) {
            true => data_image(&source).map(|bytes| (source.clone(), bytes)),
            false => {
                let local = self.base.is_some() || source.to_ascii_lowercase().starts_with("file:");
                local_image(&source, self.base.unwrap_or(Path::new(""))).filter(|_| local)
                    .and_then(|candidates| candidates.iter().find(|path| path.is_file()).and_then(|path| fs_key(path)))
                    .and_then(|path: PathBuf| std::fs::read(&path).ok().map(|bytes| (path.display().to_string(), bytes)))
            }
        };
        let index = loaded.and_then(|(key, bytes)| {
            let key = format!("{key}|{size:?}");
            if let Some(index) = self.loaded.get(&key) { return Some(*index); }
            let picture = Picture::from_bytes(&bytes, size).ok()?;
            self.parsed.pictures.push(picture);
            self.loaded.insert(key, self.parsed.pictures.len() - 1);
            Some(self.parsed.pictures.len() - 1)
        });
        match index {
            Some(index) => { out.flush(); out.blocks.push(Block::Image(index)); }
            None => { self.parsed.unreadable_images += 1; fallback(self, out); }
        }
    }

    fn table(&mut self, node: &NodeRef, style: &Computed, out: &mut Out) {
        let mut head: Vec<Vec<Span>> = Vec::new();
        let mut rows: Vec<Vec<Vec<Span>>> = Vec::new();
        let mut aligns: Vec<Align> = Vec::new();
        let mut row_nodes = Vec::new();
        for child in node.element_children() {
            let name = child.node_name().map(|name| name.to_ascii_lowercase()).unwrap_or_default();
            match name.as_str() {
                "caption" => {
                    let spans = self.inline(&child, style);
                    out.spans = spans;
                    out.flush();
                }
                "thead" | "tbody" | "tfoot" => {
                    let Some(section) = self.compute(&child, &name, style) else { continue };
                    for row in child.element_children().into_iter().filter(|row| row.node_name().is_some_and(|row| row.eq_ignore_ascii_case("tr"))) { row_nodes.push((name == "thead", row, section.clone())); }
                }
                "tr" => row_nodes.push((false, child, style.clone())),
                _ => {}
            }
        }
        for (index, (in_head, row, section)) in row_nodes.into_iter().enumerate() {
            let Some(row_style) = self.compute(&row, "tr", &section) else { continue };
            let mut cells = Vec::new();
            let mut all_headers = true;
            let mut row_aligns = Vec::new();
            for cell in row.element_children() {
                let name = cell.node_name().map(|name| name.to_ascii_lowercase()).unwrap_or_default();
                if name != "td" && name != "th" { continue; }
                all_headers &= name == "th";
                let Some(cell_style) = self.compute(&cell, &name, &row_style) else { cells.push(Vec::new()); continue };
                row_aligns.push(cell_style.align);
                cells.push(self.inline(&cell, &cell_style));
            }
            if index == 0 { aligns = row_aligns; }
            if index == 0 && (in_head || (all_headers && !cells.is_empty())) { head = cells; } else { rows.push(cells); }
        }
        if !head.is_empty() || !rows.is_empty() { out.blocks.push(Block::Table { aligns, head, rows }); }
    }
}

fn alignment(value: &str) -> Option<Align> {
    match value.trim().to_ascii_lowercase().as_str() {
        "left" | "start" | "justify" => Some(Align::Left),
        "center" | "middle" | "-webkit-center" => Some(Align::Center),
        "right" | "end" => Some(Align::Right),
        _ => None,
    }
}

fn data_image(source: &str) -> Option<Vec<u8>> {
    let (header, payload) = source[5..].split_once(',')?;
    if !header.to_ascii_lowercase().ends_with(";base64") { return None; }
    let payload = payload.chars().filter(|character| !character.is_whitespace()).collect::<String>();
    base64::engine::general_purpose::STANDARD.decode(payload).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans_text(spans: &[Span]) -> String { spans.iter().map(|span| span.text.as_str()).collect() }

    #[test]
    fn stylesheets_cascade_by_importance_specificity_and_order_with_inline_styles_last() {
        let parsed = parse(r#"<html><head><style>
            /* comment { color: blue } */
            p { color: blue; font-weight: bold }
            .note { color: #0f0 }
            #lead { color: rgb(255, 0, 0) }
            p.quiet { color: navy !important }
            @media screen and (min-width: 900px) { p { color: purple } }
            @media print { .print { font-style: italic } }
            .hidden { display: none }
        </style></head><body>
            <p id="lead" class="note">Red</p>
            <p class="note" style="color: #123456">Inline</p>
            <p class="quiet" style="color: red">Important</p>
            <p class="print">Printed</p>
            <p class="hidden">Gone</p>
        </body></html>"#, None);
        let colors = parsed.blocks.iter().map(|block| match block { Block::Paragraph(spans) => (spans_text(spans), spans[0].style.color, spans[0].style.bold, spans[0].style.italic), other => panic!("{other:?}") }).collect::<Vec<_>>();
        assert_eq!(colors, [
            ("Red".to_string(), Some([255, 0, 0]), true, false),
            ("Inline".to_string(), Some([0x12, 0x34, 0x56]), true, false),
            ("Important".to_string(), Some([0, 0, 128]), true, false),
            ("Printed".to_string(), Some([0, 0, 255]), true, true),
        ]);
    }

    #[test]
    fn structure_becomes_blocks_and_remote_resources_are_counted_once() {
        let parsed = parse(r#"<!doctype html><html><head><title> Spring
            menu </title>
            <link rel="stylesheet" href="https://cdn.example.com/site.css">
            <style>@import url("https://cdn.example.com/more.css"); @font-face { font-family: X; src: url(https://fonts.example.com/x.woff2) } body { background: #fafafa url(https://cdn.example.com/bg.png) }</style>
            <script>document.write("<p>injected</p>")</script></head>
            <body><h1 id="top">Menu</h1><h2>Menu</h2>
            <p>One<br>two <a href="https://example.com">site</a> <a href="javascript:alert(1)">nope</a></p>
            <ul><li>first</li><li>second<ol start="3"><li>nested</li></ol></li></ul>
            <table><thead><tr><th>Dish</th><th style="text-align:right">Price</th></tr></thead><tbody><tr><td>Soup</td><td>4</td></tr></tbody></table>
            <blockquote>Quoted</blockquote><pre>
  code
</pre><hr>
            <img src="https://cdn.example.com/a.png" alt="Remote A"><img src="https://cdn.example.com/a.png"><img src="//cdn.example.com/b.png">
            <iframe src="https://video.example.com/embed"></iframe><noscript>Scripts are off</noscript></body></html>"#, None);
        assert_eq!(parsed.title.as_deref(), Some("Spring menu"));
        assert_eq!(parsed.background, Some([0xfa, 0xfa, 0xfa]));
        assert_eq!(parsed.remote.images.len(), 3);
        assert_eq!(parsed.remote.stylesheets.len(), 2);
        assert_eq!(parsed.remote.fonts.len(), 1);
        assert_eq!(parsed.remote.embeds.len(), 1);
        let anchors = parsed.blocks.iter().filter_map(|block| match block { Block::Heading { anchor, .. } => Some(anchor.as_str()), _ => None }).collect::<Vec<_>>();
        assert_eq!(anchors, ["top", "menu"]);
        let Block::Paragraph(spans) = &parsed.blocks[2] else { panic!("{:?}", parsed.blocks[2]) };
        assert_eq!(spans_text(spans), "One\ntwo site nope");
        assert!(spans.iter().any(|span| span.link.as_deref() == Some("https://example.com")));
        assert!(spans.iter().all(|span| span.link.as_deref() != Some("javascript:alert(1)")));
        assert!(matches!(&parsed.blocks[3], Block::List { start: None, items } if items.len() == 2 && matches!(items[1].last(), Some(Block::List { start: Some(3), .. }))));
        assert!(matches!(&parsed.blocks[4], Block::Table { aligns, head, rows } if aligns == &[Align::Center, Align::Right] && head.len() == 2 && rows.len() == 1));
        assert!(matches!(&parsed.blocks[5], Block::Quote(blocks) if blocks.len() == 1));
        assert!(matches!(&parsed.blocks[6], Block::Code(code) if code == "  code\n"));
        assert!(matches!(parsed.blocks[7], Block::Rule));
        let text = parsed.blocks.iter().filter_map(|block| match block { Block::Paragraph(spans) => Some(spans_text(spans)), _ => None }).collect::<Vec<_>>().join("|");
        assert!(text.contains("Remote A") && text.contains("Scripts are off") && !text.contains("injected"), "{text}");
    }

    #[test]
    fn specificity_counts_ids_classes_and_types() {
        assert_eq!(specificity("p"), (0, 0, 1));
        assert_eq!(specificity("div > p.note"), (0, 1, 2));
        assert_eq!(specificity("#lead a:hover"), (1, 1, 1));
        assert_eq!(specificity("ul li[data-x=\"a b\"]"), (0, 1, 2));
    }

    #[test]
    fn fonts_map_generic_families_and_sizes() {
        let parsed = parse("<style>body{font: italic bold 18px/1.4 Georgia, serif} code{font-family: monospace} .small{font-size: 75%}</style><p>Big <code>mono</code> <span class=small>less</span></p>", None);
        let Block::Paragraph(spans) = &parsed.blocks[0] else { panic!() };
        assert!(spans[0].style.bold && spans[0].style.italic);
        assert_eq!(spans[0].style.half_points, 27);
        assert_eq!(parsed.families, ["Georgia"]);
        assert!(spans.iter().any(|span| span.text == "mono" && span.style.code));
        assert!(spans.iter().any(|span| span.text == "less" && span.style.half_points == 20));
    }
}
