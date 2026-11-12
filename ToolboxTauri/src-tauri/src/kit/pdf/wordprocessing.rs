//! The zipped-XML package reader and block model shared by the Word (DOCX), OpenDocument (ODT, ODS) and Excel (XLSX) readers.

use std::collections::HashMap;
use std::io::{Cursor, Read};

use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use zip::result::ZipError;
use zip::ZipArchive;

use crate::kit::contracts::ToolError;
use crate::kit::pdf::font_resolver::EmbeddedFont;
use crate::kit::pdf::layout::{Block, Omitted, PageSetup, Picture, Span, Style};

const MAX_DEPTH: usize = 256;
pub(super) const SHAPE_FILL: [u8; 3] = [217, 217, 217];
pub(super) const SHAPE_LINE: [u8; 3] = [89, 89, 89];
pub(super) const DEFAULT_FLOAT_SIDE: f32 = 72.;

pub struct Parsed {
    pub blocks: Vec<Block>,
    pub header: Vec<Block>,
    pub footer: Vec<Block>,
    pub header_distance: Option<f32>,
    pub footer_distance: Option<f32>,
    pub pictures: Vec<Picture>,
    pub families: Vec<String>,
    pub body_family: Option<String>,
    pub body_size: f32,
    pub embedded: Vec<EmbeddedFont>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub page: PageSetup,
    pub omitted: Omitted,
}

/// A zip package whose parts share one expansion budget; going over returns `exceeded`.
pub(super) struct Package<'a> { archive: ZipArchive<Cursor<&'a [u8]>>, budget: u64, exceeded: ToolError, damaged: &'static str }

impl<'a> Package<'a> {
    pub(super) fn open(bytes: &'a [u8], budget: u64, exceeded: ToolError, damaged: &'static str) -> Result<Self, ToolError> {
        let archive = ZipArchive::new(Cursor::new(bytes)).map_err(|_| ToolError::invalid_input(damaged))?;
        Ok(Self { archive, budget, exceeded, damaged })
    }

    pub(super) fn part(&mut self, name: &str) -> Result<Option<Vec<u8>>, ToolError> {
        let entry = match self.archive.by_name(name) {
            Ok(entry) => entry,
            Err(ZipError::FileNotFound) => return Ok(None),
            Err(_) => return Err(ToolError::invalid_input(self.damaged)),
        };
        let mut data = Vec::new();
        entry.take(self.budget.saturating_add(1)).read_to_end(&mut data).map_err(|_| ToolError::invalid_input(self.damaged))?;
        if data.len() as u64 > self.budget { return Err(self.exceeded.clone()); }
        self.budget -= data.len() as u64;
        Ok(Some(data))
    }
}

#[derive(Debug, Clone)]
pub(super) struct Rel { pub(super) target: String, pub(super) kind: String, pub(super) external: bool }

/// The relationships of a package part by id, with internal targets resolved to package paths.
pub(super) fn rels(package: &mut Package, part: &str) -> Result<HashMap<String, Rel>, ToolError> {
    let (folder, file) = part.rsplit_once('/').unwrap_or(("", part));
    let path = if folder.is_empty() { format!("_rels/{file}.rels") } else { format!("{folder}/_rels/{file}.rels") };
    let Some(bytes) = package.part(&path)? else { return Ok(HashMap::new()) };
    let root = tree(&bytes, package.damaged)?;
    Ok(root.nodes().filter(|node| node.name == "Relationship").filter_map(|node| {
        let id = node.attr("Id")?.to_string();
        let target = node.attr("Target")?;
        let external = node.attr("TargetMode").is_some_and(|mode| mode.eq_ignore_ascii_case("External"));
        let target = if external { target.trim().to_string() } else { resolve(folder, target) };
        Some((id, Rel { target, kind: node.attr("Type").unwrap_or_default().to_string(), external }))
    }).collect())
}

pub(super) fn resolve(folder: &str, target: &str) -> String {
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

#[derive(Debug, Clone, Default)]
pub(super) struct Node { pub(super) name: String, pub(super) attrs: Vec<(String, String)>, pub(super) children: Vec<Child> }

#[derive(Debug, Clone)]
pub(super) enum Child { Node(Node), Text(String) }

impl Node {
    pub(super) fn attr(&self, name: &str) -> Option<&str> { self.attrs.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_str()) }
    pub(super) fn nodes(&self) -> impl Iterator<Item = &Node> { self.children.iter().filter_map(|child| match child { Child::Node(node) => Some(node), Child::Text(_) => None }) }
    pub(super) fn child(&self, name: &str) -> Option<&Node> { self.nodes().find(|node| node.name == name) }
    pub(super) fn val(&self, name: &str) -> Option<&str> { self.child(name)?.attr("val") }
    pub(super) fn on(&self, name: &str) -> Option<bool> { self.child(name).map(|node| !matches!(node.attr("val"), Some("0" | "false" | "off" | "none"))) }
    pub(super) fn find(&self, name: &str) -> Option<&Node> { self.nodes().find_map(|node| if node.name == name { Some(node) } else { node.find(name) }) }
    pub(super) fn text(&self) -> String {
        self.children.iter().map(|child| match child { Child::Text(text) => text.clone(), Child::Node(node) => node.text() }).collect()
    }
}

pub(super) fn tree(bytes: &[u8], damaged: &'static str) -> Result<Node, ToolError> { walk(bytes, |_| false, damaged, |_| Ok(())) }

/// Builds the element tree, handing each element whose open ancestors satisfy `detach` to `each` instead of keeping it.
pub(super) fn walk(bytes: &[u8], detach: impl Fn(&[Node]) -> bool, damaged: &'static str, mut each: impl FnMut(Node) -> Result<(), ToolError>) -> Result<Node, ToolError> {
    let damaged = || ToolError::invalid_input(damaged);
    let mut reader = Reader::from_reader(bytes);
    let mut buffer = Vec::new();
    let mut stack: Vec<Node> = Vec::new();
    let mut root = None;
    let mut close = |node: Node, stack: &mut Vec<Node>| -> Result<(), ToolError> {
        if stack.is_empty() {
            root = Some(node);
        } else if detach(stack) {
            each(node)?;
        } else if let Some(parent) = stack.last_mut() {
            parent.children.push(Child::Node(node));
        }
        Ok(())
    };
    loop {
        match reader.read_event_into(&mut buffer).map_err(|_| damaged())? {
            Event::Start(start) => {
                if stack.len() >= MAX_DEPTH { return Err(damaged()); }
                stack.push(element(&start).ok_or_else(damaged)?);
            }
            Event::Empty(start) => close(element(&start).ok_or_else(damaged)?, &mut stack)?,
            Event::End(_) => {
                let node = stack.pop().ok_or_else(damaged)?;
                close(node, &mut stack)?;
            }
            Event::Text(text) => push_text(&mut stack, &text.decode().map_err(|_| damaged())?),
            Event::CData(data) => push_text(&mut stack, &data.decode().map_err(|_| damaged())?),
            Event::GeneralRef(reference) => {
                let text = match reference.resolve_char_ref().map_err(|_| damaged())? {
                    Some(character) => character.to_string(),
                    None => resolve_predefined_entity(&reference.decode().map_err(|_| damaged())?).ok_or_else(damaged)?.to_string(),
                };
                push_text(&mut stack, &text);
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    drop(close);
    if !stack.is_empty() { return Err(damaged()); }
    root.ok_or_else(damaged)
}

fn element(start: &BytesStart) -> Option<Node> {
    let name = String::from_utf8_lossy(start.local_name().as_ref()).into_owned();
    let mut attrs = Vec::new();
    for attribute in start.attributes() {
        let attribute = attribute.ok()?;
        let key = String::from_utf8_lossy(attribute.key.local_name().as_ref()).into_owned();
        let value = attribute.normalized_value(XmlVersion::Implicit1_0).ok()?.into_owned();
        attrs.push((key, value));
    }
    Some(Node { name, attrs, children: Vec::new() })
}

fn push_text(stack: &mut [Node], text: &str) {
    let Some(node) = stack.last_mut() else { return };
    match node.children.last_mut() {
        Some(Child::Text(existing)) => existing.push_str(text),
        _ => node.children.push(Child::Text(text.to_string())),
    }
}

pub(super) fn hex_color(value: &str) -> Option<[u8; 3]> {
    if value.len() != 6 { return None; }
    let channel = |index: usize| u8::from_str_radix(value.get(index..index + 2)?, 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

pub(super) fn safe_uri(target: &str) -> Option<String> {
    let target = target.trim();
    let lower = target.to_ascii_lowercase();
    ["http://", "https://", "mailto:"].iter().any(|scheme| lower.starts_with(scheme)).then(|| target.to_string())
}

pub(super) fn css_points(value: &str) -> Option<f32> {
    let split = value.find(|character: char| character.is_ascii_alphabetic()).unwrap_or(value.len());
    let number = value[..split].trim().parse::<f32>().ok().filter(|number| number.is_finite())?;
    let scale = match &value[split..] {
        "pt" => 1.,
        "in" => 72.,
        "cm" => 72. / 2.54,
        "mm" => 72. / 25.4,
        "pc" => 12.,
        "" | "px" => 0.75,
        _ => return None,
    };
    Some(number * scale)
}

/// Font families other than the body family, numbered from 1 in the order they are first used.
#[derive(Default)]
pub(super) struct Families { pub(super) body: Option<String>, pub(super) others: Vec<String> }

impl Families {
    pub(super) fn index(&mut self, font: Option<&str>) -> u16 {
        let Some(font) = font else { return 0 };
        if Some(font) == self.body.as_deref() { return 0; }
        let index = match self.others.iter().position(|family| family == font) {
            Some(index) => index,
            None => {
                self.others.push(font.to_string());
                self.others.len() - 1
            }
        };
        u16::try_from(index + 1).unwrap_or(0)
    }
}

/// A paragraph's text segments (split at page breaks), the blocks drawn after it and the floating objects anchored to it.
#[derive(Default, Clone)]
pub(super) struct Collected { pub(super) segments: Vec<Vec<Span>>, pub(super) after: Vec<Block>, pub(super) floats: Vec<Block>, pub(super) bookmark: Option<String> }

impl Collected {
    pub(super) fn new() -> Self { Self { segments: vec![Vec::new()], ..Self::default() } }

    pub(super) fn text(&mut self, text: &str, style: Style, link: Option<&str>) {
        if text.is_empty() { return; }
        let spans = self.segments.last_mut().expect("a segment always exists");
        match spans.last_mut() {
            Some(span) if span.style == style && span.link.as_deref() == link => span.text.push_str(text),
            _ => spans.push(Span { text: text.to_string(), style, link: link.map(str::to_string) }),
        }
    }
}

#[derive(Clone)]
pub(super) struct Item { pub(super) level: u8, pub(super) start: Option<u64>, pub(super) spans: Vec<Span> }

/// Collects blocks in reading order, grouping consecutive items of one list and dropping page breaks before the first block.
#[derive(Default, Clone)]
pub(super) struct Out { blocks: Vec<Block>, list: Vec<Item>, list_id: Option<String>, pending_break: bool }

impl Out {
    pub(super) fn push(&mut self, block: Block) {
        self.flush();
        self.settle();
        self.blocks.push(block);
    }

    pub(super) fn item(&mut self, id: &str, item: Item) {
        if self.list_id.as_deref() != Some(id) || self.pending_break { self.flush(); }
        self.settle();
        self.list_id = Some(id.to_string());
        self.list.push(item);
    }

    pub(super) fn page_break(&mut self) {
        self.flush();
        self.pending_break = true;
    }

    fn settle(&mut self) {
        if std::mem::take(&mut self.pending_break) && !self.blocks.is_empty() { self.blocks.push(Block::PageBreak); }
    }

    fn flush(&mut self) {
        self.list_id = None;
        let items = std::mem::take(&mut self.list);
        if !items.is_empty() { self.blocks.push(nest(&items)); }
    }

    pub(super) fn finish(mut self) -> Vec<Block> {
        self.flush();
        self.blocks
    }
}

fn nest(items: &[Item]) -> Block {
    let level = items.iter().map(|item| item.level).min().unwrap_or(0);
    let mut entries: Vec<Vec<Block>> = Vec::new();
    let mut start = None;
    let mut index = 0;
    while index < items.len() {
        if items[index].level == level {
            if entries.is_empty() { start = items[index].start; }
            entries.push(vec![Block::Paragraph(items[index].spans.clone())]);
            index += 1;
        } else {
            let end = index + items[index..].iter().take_while(|item| item.level > level).count();
            let nested = nest(&items[index..end]);
            match entries.last_mut() {
                Some(entry) => entry.push(nested),
                None => entries.push(vec![nested]),
            }
            index = end;
        }
    }
    Block::List { start, items: entries }
}
