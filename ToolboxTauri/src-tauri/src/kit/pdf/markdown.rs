use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use pulldown_cmark::{Alignment, CowStr, Event, HeadingLevel, MetadataBlockKind, Options, Parser, Tag, TagEnd};

use crate::kit::pdf::layout::{plain_text, Align, Block, Picture, Span, Style};

#[derive(Default)]
pub struct Parsed {
    pub blocks: Vec<Block>,
    pub title: Option<String>,
    pub pictures: Vec<Picture>,
    pub remote_images: usize,
    pub unreadable_images: usize,
}

enum Frame {
    Blocks(Vec<Block>),
    List { start: Option<u64>, items: Vec<Vec<Block>> },
    Table { aligns: Vec<Align>, head: Vec<Vec<Span>>, rows: Vec<Vec<Vec<Span>>>, row: Vec<Vec<Span>> },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Inline { Paragraph, Heading(u8), Cell }

struct Image { destination: String, alt: String }

struct Builder<'a> {
    base: &'a Path,
    parsed: Parsed,
    frames: Vec<Frame>,
    inline: Option<(Inline, Vec<Span>)>,
    heading_id: Option<String>,
    code: Option<String>,
    metadata: Option<String>,
    image: Option<Image>,
    bold: usize,
    italic: usize,
    strike: usize,
    links: Vec<String>,
    anchors: HashSet<String>,
    loaded: BTreeMap<PathBuf, usize>,
}

pub fn parse(text: &str, base: &Path) -> Parsed {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS | Options::ENABLE_HEADING_ATTRIBUTES;
    let mut builder = Builder {
        base, parsed: Parsed::default(), frames: vec![Frame::Blocks(Vec::new())], inline: None, heading_id: None, code: None, metadata: None, image: None,
        bold: 0, italic: 0, strike: 0, links: Vec::new(), anchors: HashSet::new(), loaded: BTreeMap::new(),
    };
    for event in Parser::new_ext(text, options) { builder.event(event); }
    builder.flush();
    let mut parsed = builder.parsed;
    if let Some(Frame::Blocks(blocks)) = builder.frames.pop() { parsed.blocks = blocks; }
    if parsed.title.is_none() {
        parsed.title = parsed.blocks.iter().find_map(|block| match block { Block::Heading { level: 1, spans, .. } => Some(plain_text(spans)), _ => None });
    }
    parsed
}

pub(crate) fn slug(text: &str) -> String {
    let mut slug = String::new();
    for character in text.trim().to_lowercase().chars() {
        if character.is_alphanumeric() || character == '-' || character == '_' { slug.push(character); }
        else if character.is_whitespace() { slug.push('-'); }
    }
    slug
}

fn strip_tags(html: &str) -> String {
    let mut text = String::new();
    let mut rest = html;
    while let Some(open) = rest.find('<') {
        text.push_str(&rest[..open]);
        let tag = &rest[open..];
        if tag.starts_with("<!--") {
            rest = tag.find("-->").map(|end| &tag[end + 3..]).unwrap_or("");
            continue;
        }
        let Some(close) = tag.find('>') else { text.push_str(tag); rest = ""; break };
        let name = tag[1..close].trim_start_matches('/').split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or_default().to_ascii_lowercase();
        if name == "br" { text.push('\n'); }
        rest = &tag[close + 1..];
    }
    text.push_str(rest);
    text.replace("&nbsp;", "\u{a0}").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&")
}

pub(crate) fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = text.get(index + 1..index + 3)?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

pub(crate) fn local_image(destination: &str, base: &Path) -> Option<Vec<PathBuf>> {
    let destination = destination.trim();
    if destination.starts_with("//") || destination.starts_with("\\\\") { return None; }
    let scheme = destination.split_once(':').map(|(scheme, _)| scheme).filter(|scheme| {
        let mut characters = scheme.chars();
        characters.next().is_some_and(|first| first.is_ascii_alphabetic()) && characters.all(|character| character.is_ascii_alphanumeric() || "+.-".contains(character))
    });
    let path = match scheme {
        Some(scheme) if scheme.eq_ignore_ascii_case("file") => {
            let rest = &destination[5..];
            let rest = rest.strip_prefix("//").map(|rest| rest.strip_prefix("localhost").unwrap_or(rest)).unwrap_or(rest);
            if !rest.starts_with('/') { return None; }
            let rest = if rest.as_bytes().get(2) == Some(&b':') { &rest[1..] } else { rest };
            return Some(vec![PathBuf::from(percent_decode(rest)?)]);
        }
        Some(scheme) if scheme.len() > 1 => return None,
        _ => destination,
    };
    let mut candidates = vec![base.join(path)];
    if let Some(decoded) = percent_decode(path).filter(|decoded| decoded != path) { candidates.push(base.join(decoded)); }
    Some(candidates)
}

impl Builder<'_> {
    fn style(&self, code: bool) -> Style { Style { bold: self.bold > 0, italic: self.italic > 0, strike: self.strike > 0, code, ..Style::default() } }

    fn push(&mut self, block: Block) {
        match self.frames.last_mut() {
            Some(Frame::Blocks(blocks)) => blocks.push(block),
            Some(Frame::List { items, .. }) => match items.last_mut() { Some(item) => item.push(block), None => items.push(vec![block]) },
            _ => {}
        }
    }

    fn flush(&mut self) {
        if let Some((Inline::Paragraph, spans)) = self.inline.take() {
            if spans.iter().any(|span| !span.text.trim().is_empty()) { self.push(Block::Paragraph(spans)); }
        }
    }

    fn span(&mut self, text: &str, code: bool) {
        if let Some(image) = &mut self.image { image.alt.push_str(text); return; }
        if let Some(code_block) = &mut self.code { code_block.push_str(text); return; }
        if let Some(metadata) = &mut self.metadata { metadata.push_str(text); return; }
        let style = self.style(code);
        let link = self.links.last().cloned();
        let (_, spans) = self.inline.get_or_insert_with(|| (Inline::Paragraph, Vec::new()));
        match spans.last_mut() {
            Some(last) if last.style == style && last.link == link => last.text.push_str(text),
            _ => spans.push(Span { text: text.to_string(), style, link }),
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

    fn picture(&mut self, image: Image) {
        let inline_context = self.inline.as_ref().map(|(kind, _)| *kind).unwrap_or(Inline::Paragraph);
        let Some(candidates) = local_image(&image.destination, self.base) else {
            self.parsed.remote_images += 1;
            if !image.alt.trim().is_empty() { self.span(&image.alt, false); }
            return;
        };
        if inline_context != Inline::Paragraph {
            if !image.alt.trim().is_empty() { self.span(&image.alt, false); }
            return;
        }
        let found = candidates.iter().find(|path| path.is_file()).and_then(|path| fs_key(path));
        let index = match found {
            Some(key) => match self.loaded.get(&key) {
                Some(index) => Some(*index),
                None => match Picture::load(&key) {
                    Ok(picture) => {
                        self.parsed.pictures.push(picture);
                        self.loaded.insert(key, self.parsed.pictures.len() - 1);
                        Some(self.parsed.pictures.len() - 1)
                    }
                    Err(_) => None,
                },
            },
            None => None,
        };
        match index {
            Some(index) => {
                self.flush();
                self.push(Block::Image(index));
            }
            None => {
                self.parsed.unreadable_images += 1;
                if !image.alt.trim().is_empty() { self.span(&image.alt, false); }
            }
        }
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.span(&text, false),
            Event::Code(text) | Event::InlineMath(text) | Event::DisplayMath(text) => self.span(&text, true),
            Event::Html(html) | Event::InlineHtml(html) => {
                let text = strip_tags(&html);
                if !text.is_empty() && (self.inline.is_some() || !text.trim().is_empty()) { self.span(&text, false); }
            }
            Event::FootnoteReference(label) => self.span(&format!("[{label}]"), false),
            Event::SoftBreak => self.span(" ", false),
            Event::HardBreak => self.span("\n", false),
            Event::Rule => { self.flush(); self.push(Block::Rule); }
            Event::TaskListMarker(checked) => self.span(if checked { "[x] " } else { "[ ] " }, false),
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => { self.flush(); self.inline = Some((Inline::Paragraph, Vec::new())); }
            Tag::Heading { level, id, .. } => {
                self.flush();
                let level = match level { HeadingLevel::H1 => 1, HeadingLevel::H2 => 2, HeadingLevel::H3 => 3, HeadingLevel::H4 => 4, HeadingLevel::H5 => 5, HeadingLevel::H6 => 6 };
                self.heading_id = id.map(|id: CowStr<'_>| id.to_string());
                self.inline = Some((Inline::Heading(level), Vec::new()));
            }
            Tag::BlockQuote(_) | Tag::FootnoteDefinition(_) | Tag::DefinitionList | Tag::DefinitionListTitle | Tag::DefinitionListDefinition | Tag::HtmlBlock => {
                self.flush();
                if matches!(tag, Tag::BlockQuote(_)) { self.frames.push(Frame::Blocks(Vec::new())); }
            }
            Tag::CodeBlock(_) => { self.flush(); self.code = Some(String::new()); }
            Tag::List(start) => { self.flush(); self.frames.push(Frame::List { start, items: Vec::new() }); }
            Tag::Item => {
                self.flush();
                if let Some(Frame::List { items, .. }) = self.frames.last_mut() { items.push(Vec::new()); }
            }
            Tag::Table(aligns) => {
                self.flush();
                let aligns = aligns.into_iter().map(|align| match align { Alignment::Center => Align::Center, Alignment::Right => Align::Right, _ => Align::Left }).collect();
                self.frames.push(Frame::Table { aligns, head: Vec::new(), rows: Vec::new(), row: Vec::new() });
            }
            Tag::TableHead | Tag::TableRow => if let Some(Frame::Table { row, .. }) = self.frames.last_mut() { row.clear(); },
            Tag::TableCell => self.inline = Some((Inline::Cell, Vec::new())),
            Tag::Emphasis => self.italic += 1,
            Tag::Strong => self.bold += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Superscript | Tag::Subscript => {}
            Tag::Link { dest_url, .. } => self.links.push(dest_url.to_string()),
            Tag::Image { dest_url, .. } => self.image = Some(Image { destination: dest_url.to_string(), alt: String::new() }),
            Tag::MetadataBlock(kind) => self.metadata = (kind == MetadataBlockKind::YamlStyle).then(String::new),
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.flush(),
            TagEnd::Heading(_) => {
                if let Some((Inline::Heading(level), spans)) = self.inline.take() {
                    let preferred = self.heading_id.take();
                    let anchor = self.anchor(preferred, &spans);
                    self.push(Block::Heading { level, spans, anchor });
                }
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                if let Some(Frame::Blocks(blocks)) = self.frames.pop() { self.push(Block::Quote(blocks)); }
            }
            TagEnd::FootnoteDefinition | TagEnd::DefinitionList | TagEnd::DefinitionListTitle | TagEnd::DefinitionListDefinition | TagEnd::HtmlBlock => self.flush(),
            TagEnd::CodeBlock => if let Some(code) = self.code.take() { self.push(Block::Code(code)); },
            TagEnd::List(_) => {
                self.flush();
                if let Some(Frame::List { start, items }) = self.frames.pop() { self.push(Block::List { start, items }); }
            }
            TagEnd::Item => self.flush(),
            TagEnd::Table => if let Some(Frame::Table { aligns, head, rows, .. }) = self.frames.pop() { self.push(Block::Table { aligns, head, rows }); },
            TagEnd::TableHead => if let Some(Frame::Table { head, row, .. }) = self.frames.last_mut() { *head = std::mem::take(row); },
            TagEnd::TableRow => if let Some(Frame::Table { rows, row, .. }) = self.frames.last_mut() { rows.push(std::mem::take(row)); },
            TagEnd::TableCell => {
                let spans = self.inline.take().map(|(_, spans)| spans).unwrap_or_default();
                if let Some(Frame::Table { row, .. }) = self.frames.last_mut() { row.push(spans); }
            }
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Superscript | TagEnd::Subscript => {}
            TagEnd::Link => { self.links.pop(); }
            TagEnd::Image => if let Some(image) = self.image.take() { self.picture(image); },
            TagEnd::MetadataBlock(_) => {
                if let Some(metadata) = self.metadata.take() {
                    self.parsed.title = metadata.lines().find_map(|line| line.strip_prefix("title:")).map(|title| title.trim().trim_matches(|c| c == '"' || c == '\'').to_string()).filter(|title| !title.is_empty());
                }
            }
        }
    }
}

pub(crate) fn fs_key(path: &Path) -> Option<PathBuf> { std::fs::canonicalize(path).ok() }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destinations_split_into_local_candidates_and_remote_links() {
        let base = Path::new("/notes");
        assert_eq!(local_image("img/a b.png", base), Some(vec![base.join("img/a b.png")]));
        assert_eq!(local_image("img/a%20b.png", base), Some(vec![base.join("img/a%20b.png"), base.join("img/a b.png")]));
        assert_eq!(local_image("file:///tmp/x.png", base), Some(vec![PathBuf::from("/tmp/x.png")]));
        for remote in ["https://example.com/a.png", "http://127.0.0.1/a.png", "//cdn.example.com/a.png", "data:image/png;base64,AAAA", "ftp://host/a.png"] {
            assert_eq!(local_image(remote, base), None, "{remote}");
        }
    }

    #[test]
    fn headings_lists_tables_and_quotes_become_blocks_with_unique_anchors() {
        let parsed = parse("---\ntitle: \"Release notes\"\n---\n# Intro\n\n- one\n- **two** [site](https://example.com)\n\n> quoted\n\n| A | B |\n|:-|-:|\n| 1 | 2 |\n\n# Intro\n\nline<br>break <!-- hidden -->\n", Path::new("."));
        assert_eq!(parsed.title.as_deref(), Some("Release notes"));
        let anchors = parsed.blocks.iter().filter_map(|block| match block { Block::Heading { anchor, .. } => Some(anchor.as_str()), _ => None }).collect::<Vec<_>>();
        assert_eq!(anchors, ["intro", "intro-1"]);
        assert!(matches!(&parsed.blocks[1], Block::List { start: None, items } if items.len() == 2));
        assert!(matches!(&parsed.blocks[2], Block::Quote(blocks) if blocks.len() == 1));
        assert!(matches!(&parsed.blocks[3], Block::Table { aligns, head, rows } if aligns == &[Align::Left, Align::Right] && head.len() == 2 && rows.len() == 1));
        let Block::Paragraph(spans) = &parsed.blocks[5] else { panic!("{:?}", parsed.blocks[5]) };
        assert_eq!(spans.iter().map(|span| span.text.as_str()).collect::<String>().trim(), "line\nbreak");
    }
}
