//! EPUB books on the HTML path. Chapters are read in spine order, with their stylesheets and images
//! taken from inside the package only, and the navigation document (EPUB 3 nav, or the EPUB 2 NCX)
//! becomes the bookmarks. Copy-protected books are refused; obfuscated fonts alone are not protection.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use dom_query::{Document, NodeRef};

use crate::kit::contracts::{LimitKind, ToolError};
use crate::kit::pdf::html::{self, LeftOut, Resources};
use crate::kit::pdf::layout::{Block, Picture, Span};
use crate::kit::pdf::markdown::percent_decode;
use crate::kit::pdf::wordprocessing::{resolve, tree, Node, Package};

/// Spine entries read from one book; a longer spine is refused rather than laid out.
pub const MAX_SPINE_ITEMS: usize = 5_000;
/// Navigation entries kept as bookmarks; any after these are left out.
pub const MAX_NAV_ENTRIES: usize = 10_000;
/// Nesting kept from the navigation; deeper entries are left out.
pub const MAX_NAV_DEPTH: usize = 16;
const DAMAGED: &str = "This eBook is damaged or isn't a valid EPUB.";
pub const PROTECTED: &str = "This eBook is copy-protected (DRM), so it can't be converted. Only eBooks without copy protection can become PDFs.";
/// Rights files that copy-protection schemes (Adobe, Readium LCP, Apple) add to the package.
const RIGHTS_FILES: &[&str] = &["META-INF/rights.xml", "META-INF/license.lcpl", "META-INF/sinf.xml"];
/// Encryption that only obfuscates embedded fonts, which readers undo without a key.
const FONT_OBFUSCATION: &[&str] = &["http://www.idpf.org/2008/embedding", "http://ns.adobe.com/pdf/enc#RC"];
const CHAPTER_TYPES: &[&str] = &["application/xhtml+xml", "text/html"];

/// A spine document; heading anchors are `{path}#{id}` and in-book links point at them.
pub struct Chapter { pub path: String, pub blocks: Vec<Block> }

/// A navigation entry; `target` is a chapter path, with `#{id}` when it points inside the chapter.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry { pub level: u8, pub title: String, pub target: String }

#[derive(Default)]
pub struct Book {
    pub chapters: Vec<Chapter>,
    pub pictures: Vec<Picture>,
    pub families: Vec<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub navigation: Vec<Entry>,
    pub remote: LeftOut,
    pub unreadable_images: usize,
    pub unreadable_stylesheets: usize,
}

impl Book {
    /// Adds a parsed chapter at the end: its pictures, fonts and left-out content join the book's, its
    /// heading anchors become `{path}#{id}` (and are added to `anchors`), and `scale` multiplies its text sizes.
    pub(super) fn absorb(&mut self, path: String, mut parsed: html::Parsed, scale: f32, anchors: &mut HashSet<String>) {
        let (offset, families) = (self.pictures.len(), merged(&mut self.families, &parsed.families));
        visit(&mut parsed.blocks, &mut |block| match block {
            Block::Heading { anchor, .. } => { *anchor = format!("{path}#{anchor}"); anchors.insert(anchor.clone()); }
            Block::Image(index) => *index += offset,
            Block::Float(float) => if let Some(index) = &mut float.picture { *index += offset },
            Block::Formatted(paragraph) => { paragraph.size *= scale; paragraph.before *= scale; paragraph.after *= scale; }
            _ => {}
        }, &mut |span| {
            if span.style.family > 0 { span.style.family = families[span.style.family as usize - 1]; }
            if span.style.half_points > 0 { span.style.half_points = (f32::from(span.style.half_points) * scale).round().clamp(1., f32::from(u16::MAX)) as u16; }
        });
        self.pictures.append(&mut parsed.pictures);
        self.remote.images.append(&mut parsed.remote.images);
        self.remote.stylesheets.append(&mut parsed.remote.stylesheets);
        self.remote.fonts.append(&mut parsed.remote.fonts);
        self.remote.embeds.append(&mut parsed.remote.embeds);
        self.unreadable_images += parsed.unreadable_images;
        self.unreadable_stylesheets += parsed.unreadable_stylesheets;
        self.chapters.push(Chapter { path, blocks: parsed.blocks });
    }
}

struct Item { path: String, media: String, properties: String }

/// Reads an EPUB. `budget` caps the bytes expanded across every part read and going over returns
/// `exceeded`; `scale` multiplies the book's text sizes.
pub fn parse(bytes: &[u8], budget: u64, exceeded: ToolError, scale: f32) -> Result<Book, ToolError> {
    let mut package = Package::open(bytes, budget, exceeded, DAMAGED)?;
    refuse_protected(&mut package)?;
    let container = package.part("META-INF/container.xml")?.ok_or_else(damaged)?;
    let container = tree(&container, DAMAGED)?;
    let opf_path = container.find("rootfile").and_then(|node| node.attr("full-path")).map(|path| resolve("", path)).ok_or_else(damaged)?;
    let opf = tree(&package.part(&opf_path)?.ok_or_else(damaged)?, DAMAGED)?;
    let folder = folder_of(&opf_path).to_string();

    let metadata = opf.child("metadata");
    let first = |name: &str| metadata.and_then(|metadata| metadata.find(name)).map(|node| node.text().split_whitespace().collect::<Vec<_>>().join(" ")).filter(|text| !text.is_empty());
    let mut book = Book { title: first("title"), author: first("creator"), ..Book::default() };

    let manifest: HashMap<String, Item> = opf.child("manifest").map(|manifest| manifest.nodes().filter(|node| node.name == "item").filter_map(|item| {
        let href = item.attr("href")?;
        let path = resolve(&folder, &percent_decode(href).unwrap_or_else(|| href.to_string()));
        Some((item.attr("id")?.to_string(), Item { path, media: item.attr("media-type").unwrap_or_default().to_string(), properties: item.attr("properties").unwrap_or_default().to_string() }))
    }).collect()).unwrap_or_default();
    let spine = opf.child("spine").ok_or_else(damaged)?;
    let mut paths = Vec::new();
    for itemref in spine.nodes().filter(|node| node.name == "itemref") {
        let Some(item) = itemref.attr("idref").and_then(|id| manifest.get(id)) else { continue };
        if !CHAPTER_TYPES.contains(&item.media.as_str()) || paths.contains(&item.path) { continue; }
        if paths.len() == MAX_SPINE_ITEMS {
            return Err(ToolError::limit_exceeded(LimitKind::PageCount, format!("This eBook has more than {MAX_SPINE_ITEMS} chapters, the most that can be converted at once.")));
        }
        paths.push(item.path.clone());
    }
    if paths.is_empty() { return Err(ToolError::invalid_input("This eBook has no chapters to convert.")); }

    let mut store = Store { package, cache: BTreeMap::new(), error: None };
    let mut anchors = HashSet::new();
    for path in &paths {
        let markup = store.part(path)?.ok_or_else(damaged)?;
        let base = PathBuf::from(folder_of(path));
        let parsed = html::parse_with(&String::from_utf8_lossy(&markup), Some(&base), &mut store);
        if let Some(error) = store.error.take() { return Err(error); }
        book.absorb(path.clone(), parsed, scale, &mut anchors);
    }

    let chapters = paths.iter().cloned().collect::<HashSet<_>>();
    let target = |from: &str, href: &str| in_book(from, href, &chapters, &anchors);
    for chapter in &mut book.chapters {
        let folder = folder_of(&chapter.path).to_string();
        let path = chapter.path.clone();
        visit(&mut chapter.blocks, &mut |_| {}, &mut |span| {
            let Some(link) = span.link.take() else { return };
            span.link = if link.starts_with('#') { target(&folder, &format!("{}{link}", file_of(&path))) } else if has_scheme(&link) { web(&link).then_some(link) } else { target(&folder, &link) };
        });
    }

    let nav = manifest.values().find(|item| item.properties.split_whitespace().any(|property| property == "nav"));
    book.navigation = match nav {
        Some(item) => match store.part(&item.path)? {
            Some(markup) => nav_entries(&String::from_utf8_lossy(&markup), folder_of(&item.path)),
            None => Vec::new(),
        },
        None => Vec::new(),
    };
    if book.navigation.is_empty() {
        let ncx = spine.attr("toc").and_then(|id| manifest.get(id)).or_else(|| manifest.values().find(|item| item.media == "application/x-dtbncx+xml"));
        if let Some(item) = ncx {
            if let Some(bytes) = store.part(&item.path)? {
                let ncx = tree(&bytes, DAMAGED)?;
                if let Some(map) = ncx.child("navMap") { ncx_entries(map, folder_of(&item.path), 1, &mut book.navigation); }
            }
        }
    }
    Ok(book)
}

fn damaged() -> ToolError { ToolError::invalid_input(DAMAGED) }

fn folder_of(path: &str) -> &str { path.rsplit_once('/').map(|(folder, _)| folder).unwrap_or("") }

fn file_of(path: &str) -> &str { path.rsplit_once('/').map(|(_, file)| file).unwrap_or(path) }

/// Links that leave the book are kept only for the web and mail.
pub(super) fn web(href: &str) -> bool {
    let lower = href.to_ascii_lowercase();
    ["http://", "https://", "mailto:"].iter().any(|scheme| lower.starts_with(scheme))
}

fn has_scheme(href: &str) -> bool { href.starts_with("//") || href.split(['/', '?', '#']).next().is_some_and(|head| head.contains(':')) }

/// A book is protected when a rights file is present or anything besides fonts is encrypted.
fn refuse_protected(package: &mut Package) -> Result<(), ToolError> {
    for name in RIGHTS_FILES {
        if package.part(name)?.is_some() { return Err(ToolError::invalid_input(PROTECTED)); }
    }
    let Some(encryption) = package.part("META-INF/encryption.xml")? else { return Ok(()) };
    let encryption = tree(&encryption, DAMAGED)?;
    let mut methods = Vec::new();
    collect(&encryption, "EncryptionMethod", &mut methods);
    if methods.iter().any(|method| !FONT_OBFUSCATION.contains(&method.attr("Algorithm").unwrap_or_default())) {
        return Err(ToolError::invalid_input(PROTECTED));
    }
    Ok(())
}

fn collect<'n>(node: &'n Node, name: &str, found: &mut Vec<&'n Node>) {
    for child in node.nodes() {
        if child.name == name { found.push(child); }
        collect(child, name, found);
    }
}

/// Adds a chapter's font families to the book's, returning where each now sits (as a style's 1-based family).
fn merged(book: &mut Vec<String>, chapter: &[String]) -> Vec<u16> {
    chapter.iter().map(|family| {
        let index = book.iter().position(|known| known == family).unwrap_or_else(|| { book.push(family.clone()); book.len() - 1 });
        index as u16 + 1
    }).collect()
}

/// Calls `each_block` on every block, nested ones included, and `each_span` on every span of text.
pub(super) fn visit(blocks: &mut [Block], each_block: &mut dyn FnMut(&mut Block), each_span: &mut dyn FnMut(&mut Span)) {
    for block in blocks {
        each_block(block);
        match block {
            Block::Heading { spans, .. } | Block::Paragraph(spans) => spans.iter_mut().for_each(&mut *each_span),
            Block::Formatted(paragraph) => paragraph.spans.iter_mut().for_each(&mut *each_span),
            Block::Quote(blocks) => visit(blocks, each_block, each_span),
            Block::Float(float) => visit(&mut float.blocks, each_block, each_span),
            Block::List { items, .. } => items.iter_mut().for_each(|item| visit(item, each_block, each_span)),
            Block::Table { head, rows, .. } => head.iter_mut().chain(rows.iter_mut().flatten()).flatten().for_each(&mut *each_span),
            Block::Code(_) | Block::Image(_) | Block::Rule | Block::PageBreak => {}
        }
    }
}

/// Where a local link inside the book lands: its heading anchor when there is one, otherwise the chapter's start.
fn in_book(folder: &str, href: &str, chapters: &HashSet<String>, anchors: &HashSet<String>) -> Option<String> {
    let (target, fragment) = href.split_once('#').unwrap_or((href, ""));
    let target = target.split('?').next().unwrap_or_default();
    let path = resolve(folder, &percent_decode(target).unwrap_or_else(|| target.to_string()));
    if !chapters.contains(&path) { return None; }
    let anchor = format!("{path}#{}", percent_decode(fragment).unwrap_or_else(|| fragment.to_string()));
    Some(if !fragment.is_empty() && anchors.contains(&anchor) { format!("#{anchor}") } else { format!("#{path}") })
}

/// A navigation target as a chapter path, with `#{id}` when it names a place inside the chapter.
fn nav_target(folder: &str, href: &str) -> Option<String> {
    let href = href.trim();
    if href.is_empty() || has_scheme(href) { return None; }
    let (target, fragment) = href.split_once('#').unwrap_or((href, ""));
    let target = target.split('?').next().unwrap_or_default();
    let path = resolve(folder, &percent_decode(target).unwrap_or_else(|| target.to_string()));
    let fragment = percent_decode(fragment).unwrap_or_else(|| fragment.to_string());
    Some(if fragment.is_empty() { path } else { format!("{path}#{fragment}") })
}

/// The EPUB 3 table of contents: the `nav` marked as the toc, or the first one, read as nested lists.
fn nav_entries(markup: &str, folder: &str) -> Vec<Entry> {
    let document = Document::from(markup);
    let navs = document.select("nav").nodes().to_vec();
    let is_toc = |nav: &NodeRef| nav.attr("epub:type").is_some_and(|kind| kind.split_whitespace().any(|kind| kind == "toc"));
    let Some(nav) = navs.iter().find(|nav| is_toc(nav)).or(navs.first()) else { return Vec::new() };
    let mut entries = Vec::new();
    if let Some(list) = nav.element_children().into_iter().find(|child| named(child, "ol") || named(child, "ul")) { nav_list(&list, folder, 1, &mut entries); }
    entries
}

fn named(node: &NodeRef, name: &str) -> bool { node.node_name().is_some_and(|node| node.eq_ignore_ascii_case(name)) }

fn nav_list(list: &NodeRef, folder: &str, level: usize, entries: &mut Vec<Entry>) {
    if level > MAX_NAV_DEPTH { return; }
    for item in list.element_children().into_iter().filter(|child| named(child, "li")) {
        if entries.len() == MAX_NAV_ENTRIES { return; }
        let children = item.element_children();
        let label = children.iter().find(|child| named(child, "a") || named(child, "span"));
        let title = label.map(|label| label.text().split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default();
        let target = label.and_then(|label| label.attr("href")).and_then(|href| nav_target(folder, &href));
        if let (false, Some(target)) = (title.is_empty(), target) { entries.push(Entry { level: level as u8, title, target }); }
        if let Some(nested) = children.iter().find(|child| named(child, "ol") || named(child, "ul")) { nav_list(nested, folder, level + 1, entries); }
    }
}

/// The EPUB 2 table of contents: nested `navPoint`s in the NCX.
fn ncx_entries(parent: &Node, folder: &str, level: usize, entries: &mut Vec<Entry>) {
    if level > MAX_NAV_DEPTH { return; }
    for point in parent.nodes().filter(|node| node.name == "navPoint") {
        if entries.len() == MAX_NAV_ENTRIES { return; }
        let title = point.child("navLabel").map(|label| label.text().split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default();
        let target = point.child("content").and_then(|content| content.attr("src")).and_then(|src| nav_target(folder, src));
        if let (false, Some(target)) = (title.is_empty(), target) { entries.push(Entry { level: level as u8, title, target }); }
        ncx_entries(point, folder, level + 1, entries);
    }
}

/// Package parts read once each, so a stylesheet or image every chapter uses counts against the budget once.
struct Store<'a> { package: Package<'a>, cache: BTreeMap<String, Vec<u8>>, error: Option<ToolError> }

impl Store<'_> {
    fn part(&mut self, name: &str) -> Result<Option<Vec<u8>>, ToolError> {
        if let Some(bytes) = self.cache.get(name) { return Ok(Some(bytes.clone())); }
        let bytes = self.package.part(name)?;
        if let Some(bytes) = &bytes { self.cache.insert(name.to_string(), bytes.clone()); }
        Ok(bytes)
    }
}

impl Resources for Store<'_> {
    /// Only parts of the package: remote addresses, `file:` paths and anything outside it are never read.
    fn read(&mut self, href: &str, base: Option<&Path>) -> Option<(PathBuf, Vec<u8>)> {
        let href = href.trim();
        if self.error.is_some() || href.is_empty() || has_scheme(href) { return None; }
        let target = href.split(['#', '?']).next().unwrap_or_default();
        let folder = base.map(|base| base.to_string_lossy().replace('\\', "/")).unwrap_or_default();
        let name = resolve(&folder, &percent_decode(target).unwrap_or_else(|| target.to_string()));
        match self.part(&name) {
            Ok(bytes) => bytes.map(|bytes| (PathBuf::from(name), bytes)),
            Err(error) => { self.error = Some(error); None }
        }
    }
}
