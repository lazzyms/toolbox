//! MOBI (Mobipocket) books on the eBook path, in basic fidelity. The PalmDB records are read and
//! decompressed (none, PalmDOC LZ77 or HUFF/CDIC), the text is split into chapters at its page breaks,
//! `recindex` images come from the image records, `filepos` links land on the chapter that holds their
//! target, and the guide's table of contents becomes the bookmarks. Copy-protected books are refused,
//! and so are books that hold only the newer Kindle Format 8 (AZW3) text.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use dom_query::Document;
use encoding_rs::{UTF_8, WINDOWS_1252};

use crate::kit::contracts::{LimitKind, ToolError};
use crate::kit::pdf::epub::{self, Book, Entry, MAX_NAV_ENTRIES, MAX_SPINE_ITEMS, PROTECTED};
use crate::kit::pdf::html::{self, Resources};

const DAMAGED: &str = "This eBook is damaged or isn't a valid MOBI.";
const KF8_ONLY: &str = "This eBook is in Kindle Format 8 (AZW3), which can't be converted yet. Only MOBI books in the older Mobipocket format can become PDFs.";
const NO_COMPRESSION: u16 = 1;
const PALMDOC: u16 = 2;
const HUFF_CDIC: u16 = 17_480;
const NONE: u32 = u32::MAX;
/// Nesting allowed when a HUFF/CDIC phrase is itself compressed; deeper chains are treated as damage.
const MAX_PHRASE_DEPTH: usize = 32;
const PAGE_BREAK: &[u8] = b"<mbp:pagebreak";

/// Reads a MOBI book. `budget` caps the decompressed text and going over returns `exceeded`; `scale`
/// multiplies the book's text sizes.
pub fn parse(bytes: &[u8], budget: u64, exceeded: ToolError, scale: f32) -> Result<Book, ToolError> {
    let records = records(bytes)?;
    let header = Header::read(records[0])?;
    if header.protected { return Err(ToolError::invalid_input(PROTECTED)); }
    if header.version >= 8 { return Err(ToolError::invalid_input(KF8_ONLY)); }
    if u64::from(header.text_length) > budget { return Err(exceeded); }

    let mut text = Vec::new();
    let mut huff = match header.compression {
        HUFF_CDIC => Some(Huff::load(&records, header.huff_record, header.huff_count)?),
        NO_COMPRESSION | PALMDOC => None,
        _ => return Err(damaged()),
    };
    for index in 1..=usize::from(header.text_records) {
        let record = records.get(index).ok_or_else(damaged)?;
        let record = &record[..record.len() - trailing(record, header.extra_flags)];
        let room = budget - text.len() as u64;
        let before = text.len();
        let outcome = match (&mut huff, header.compression) {
            (Some(huff), _) => huff.unpack(record, &mut text, room, 0),
            (None, PALMDOC) => palmdoc(record, &mut text, room),
            _ => { text.extend_from_slice(record); Ok(()) }
        };
        match outcome {
            Ok(()) if (text.len() - before) as u64 <= room => {}
            Ok(()) | Err(Unpack::TooBig) => return Err(exceeded),
            Err(Unpack::Damaged) => return Err(damaged()),
        }
    }
    text.truncate(header.text_length as usize);

    let decode = |raw: &[u8]| -> String {
        let encoding = if header.utf8 { UTF_8 } else { WINDOWS_1252 };
        encoding.decode_without_bom_handling(raw).0.into_owned()
    };
    let mut book = Book { title: header.title.as_deref().map(&decode), author: header.author.as_deref().map(&decode), ..Book::default() };
    book.title = book.title.map(|title| tidy(&title)).filter(|title| !title.is_empty()).or_else(|| Some(tidy(&decode(header.full_name(records[0])))).filter(|title| !title.is_empty()));
    book.author = book.author.map(|author| tidy(&author)).filter(|author| !author.is_empty());

    // Chapters run from one page break to the next; links and the table of contents point at byte offsets.
    let mut starts = vec![0];
    starts.extend(find_all(&text, PAGE_BREAK).into_iter().filter(|start| *start > 0));
    if starts.len() > MAX_SPINE_ITEMS {
        return Err(ToolError::limit_exceeded(LimitKind::PageCount, format!("This eBook has more than {MAX_SPINE_ITEMS} chapters, the most that can be converted at once.")));
    }
    let chapter_at = |offset: u64| starts.partition_point(|start| *start as u64 <= offset).saturating_sub(1);
    let toc = guide_toc(&text, &decode);

    let mut images = Images { records: &records, first: header.first_image };
    let mut anchors = HashSet::new();
    let mut kept = Vec::new();
    for (index, start) in starts.iter().enumerate() {
        let end = starts.get(index + 1).copied().unwrap_or(text.len());
        let raw = &text[*start..end];
        if let Some(toc) = toc.filter(|toc| (*start as u64..end as u64).contains(toc)) {
            book.navigation = navigation(&decode(&raw[(toc as usize - start)..]), &chapter_at);
        }
        let document = Document::from(decode(raw).as_str());
        for image in document.select("img[recindex]").nodes() {
            let record = image.attr("recindex").unwrap_or_default();
            image.set_attr("src", &format!("{IMAGE_PREFIX}{}", record.trim()));
        }
        for link in document.select("a[filepos]").nodes() {
            match link.attr("filepos").and_then(|offset| offset.trim().parse::<u64>().ok()) {
                Some(offset) => link.set_attr("href", &format!("#{}", part(chapter_at(offset)))),
                None => link.remove_attr("filepos"),
            }
        }
        let parsed = html::parse_with(&document.html(), Some(Path::new("")), &mut images);
        if parsed.blocks.is_empty() { continue; }
        kept.push(index);
        book.absorb(part(index), parsed, scale, &mut anchors);
    }
    if book.chapters.is_empty() { return Err(ToolError::invalid_input("This eBook has no chapters to convert.")); }

    // A link or bookmark to a chapter with nothing in it lands on the next chapter that has content.
    let landing = |index: usize| kept.get(kept.partition_point(|kept| *kept < index)).map(|index| part(*index));
    let chapter_of = |target: &str| target.strip_prefix(PART).and_then(|index| index.parse::<usize>().ok());
    for chapter in &mut book.chapters {
        epub::visit(&mut chapter.blocks, &mut |_| {}, &mut |span| {
            let Some(link) = span.link.take() else { return };
            span.link = match link.strip_prefix('#') {
                Some(target) if anchors.contains(target) => Some(link),
                Some(target) => chapter_of(target).and_then(landing).map(|path| format!("#{path}")),
                None => epub::web(&link).then_some(link),
            };
        });
    }
    book.navigation = std::mem::take(&mut book.navigation).into_iter().filter_map(|entry| {
        let target = chapter_of(&entry.target).and_then(landing)?;
        Some(Entry { target, ..entry })
    }).collect();
    Ok(book)
}

const PART: &str = "part-";
const IMAGE_PREFIX: &str = "record/";

fn part(index: usize) -> String { format!("{PART}{index}") }

fn damaged() -> ToolError { ToolError::invalid_input(DAMAGED) }

fn tidy(text: &str) -> String { text.split_whitespace().collect::<Vec<_>>().join(" ") }

fn u16_at(data: &[u8], offset: usize) -> Option<u16> { Some(u16::from_be_bytes(data.get(offset..offset + 2)?.try_into().ok()?)) }

fn u32_at(data: &[u8], offset: usize) -> Option<u32> { Some(u32::from_be_bytes(data.get(offset..offset + 4)?.try_into().ok()?)) }

/// The PalmDB records, in order; record 0 holds the book's headers.
fn records(bytes: &[u8]) -> Result<Vec<&[u8]>, ToolError> {
    if bytes.get(60..68) != Some(b"BOOKMOBI") { return Err(damaged()); }
    let count = usize::from(u16_at(bytes, 76).ok_or_else(damaged)?);
    if count == 0 { return Err(damaged()); }
    let offsets = (0..count).map(|index| u32_at(bytes, 78 + index * 8).map(|offset| offset as usize).ok_or_else(damaged)).collect::<Result<Vec<_>, _>>()?;
    offsets.iter().enumerate().map(|(index, start)| {
        let end = offsets.get(index + 1).copied().unwrap_or(bytes.len());
        bytes.get(*start..end).ok_or_else(damaged)
    }).collect()
}

#[derive(Debug)]
struct Header {
    compression: u16,
    text_length: u32,
    text_records: u16,
    protected: bool,
    utf8: bool,
    version: u32,
    full_name: (u32, u32),
    first_image: u32,
    huff_record: u32,
    huff_count: u32,
    extra_flags: u16,
    title: Option<Vec<u8>>,
    author: Option<Vec<u8>>,
}

impl Header {
    fn read(record: &[u8]) -> Result<Self, ToolError> {
        let field = |offset| u32_at(record, offset).ok_or_else(damaged);
        if record.get(16..20) != Some(b"MOBI") { return Err(damaged()); }
        let length = field(20)? as usize;
        let optional = |offset: usize| if offset + 4 <= 16 + length { u32_at(record, offset) } else { None };
        let encryption = u16_at(record, 12).ok_or_else(damaged)?;
        let drm = optional(168).filter(|offset| *offset != NONE && *offset != 0).is_some() && optional(172).is_some_and(|count| count > 0 && count != NONE);
        let mut header = Header {
            compression: u16_at(record, 0).ok_or_else(damaged)?,
            text_length: field(4)?,
            text_records: u16_at(record, 8).ok_or_else(damaged)?,
            protected: encryption != 0 || drm,
            utf8: field(28)? == 65_001,
            version: optional(36).unwrap_or(0),
            full_name: (optional(84).unwrap_or(0), optional(88).unwrap_or(0)),
            first_image: optional(108).unwrap_or(NONE),
            huff_record: optional(112).unwrap_or(0),
            huff_count: optional(116).unwrap_or(0),
            extra_flags: if length >= 0xE4 { u16_at(record, 0xF2).unwrap_or(0) } else { 0 },
            title: None,
            author: None,
        };
        if optional(128).is_some_and(|flags| flags & 0x40 != 0) { header.exth(record.get(16 + length..).unwrap_or_default()); }
        Ok(header)
    }

    /// Title (503) and author (100) from the EXTH metadata; a malformed block is ignored.
    fn exth(&mut self, data: &[u8]) {
        if data.get(..4) != Some(b"EXTH") { return; }
        let Some(count) = u32_at(data, 8) else { return };
        let mut offset = 12;
        for _ in 0..count {
            let (Some(kind), Some(length)) = (u32_at(data, offset), u32_at(data, offset + 4)) else { return };
            let Some(value) = (length as usize).checked_sub(8).and_then(|size| data.get(offset + 8..offset + 8 + size)) else { return };
            match kind {
                100 if self.author.is_none() => self.author = Some(value.to_vec()),
                503 if self.title.is_none() => self.title = Some(value.to_vec()),
                _ => {}
            }
            offset += length as usize;
        }
    }

    fn full_name<'r>(&self, record: &'r [u8]) -> &'r [u8] {
        let (offset, length) = (self.full_name.0 as usize, self.full_name.1 as usize);
        record.get(offset..offset.saturating_add(length)).unwrap_or_default()
    }
}

/// Bytes at the end of a text record that aren't text: the entries `extra_flags` announces.
fn trailing(record: &[u8], extra_flags: u16) -> usize {
    let mut size = 0;
    let mut flags = extra_flags >> 1;
    while flags != 0 {
        if flags & 1 != 0 { size += entry_size(&record[..record.len() - size.min(record.len())]); }
        flags >>= 1;
    }
    if extra_flags & 1 != 0 {
        if let Some(last) = record.len().checked_sub(size + 1).map(|index| record[index]) { size += usize::from(last & 3) + 1; }
    }
    size.min(record.len())
}

/// A trailing entry's size, written backwards from the end as a variable-length number.
fn entry_size(data: &[u8]) -> usize {
    let (mut size, mut shift) = (0, 0);
    for byte in data.iter().rev() {
        size |= usize::from(byte & 0x7F) << shift;
        shift += 7;
        if byte & 0x80 != 0 || shift >= 28 { break; }
    }
    size
}

#[derive(Debug, PartialEq)]
enum Unpack { Damaged, TooBig }

/// PalmDOC's LZ77: literals, runs of raw bytes, back references and space-plus-character pairs.
fn palmdoc(data: &[u8], out: &mut Vec<u8>, room: u64) -> Result<(), Unpack> {
    let start = out.len();
    let mut index = 0;
    while index < data.len() {
        let byte = data[index];
        index += 1;
        match byte {
            0x01..=0x08 => {
                let run = data.get(index..index + usize::from(byte)).ok_or(Unpack::Damaged)?;
                out.extend_from_slice(run);
                index += usize::from(byte);
            }
            0x00 | 0x09..=0x7F => out.push(byte),
            0x80..=0xBF => {
                let next = *data.get(index).ok_or(Unpack::Damaged)?;
                index += 1;
                let pair = (usize::from(byte) << 8 | usize::from(next)) & 0x3FFF;
                let (distance, length) = (pair >> 3, (pair & 7) + 3);
                if distance == 0 || distance > out.len() - start { return Err(Unpack::Damaged); }
                for _ in 0..length { out.push(out[out.len() - distance]); }
            }
            0xC0..=0xFF => { out.push(b' '); out.push(byte ^ 0x80); }
        }
        if (out.len() - start) as u64 > room { return Err(Unpack::TooBig); }
    }
    Ok(())
}

enum Phrase { Literal(Vec<u8>), Packed(Vec<u8>), Unpacking }

/// The HUFF/CDIC compression used by larger Mobipocket books: Huffman codes for phrases from the
/// CDIC dictionaries, where a phrase may itself be compressed.
struct Huff { dict1: Vec<(u32, bool, u64)>, mincode: [u64; 33], maxcode: [u64; 33], phrases: Vec<Phrase> }

impl Huff {
    fn load(records: &[&[u8]], first: u32, count: u32) -> Result<Self, ToolError> {
        let first = first as usize;
        let huff = records.get(first).filter(|huff| huff.starts_with(b"HUFF\0\0\0\x18")).ok_or_else(damaged)?;
        let (off1, off2) = (u32_at(huff, 8).ok_or_else(damaged)? as usize, u32_at(huff, 12).ok_or_else(damaged)? as usize);
        let dict1 = (0..256).map(|index| {
            let value = u32_at(huff, off1 + index * 4).ok_or_else(damaged)?;
            let length = value & 0x1F;
            if length == 0 { return Err(damaged()); }
            Ok((length, value & 0x80 != 0, ((u64::from(value >> 8) + 1) << (32 - length)) - 1))
        }).collect::<Result<Vec<_>, _>>()?;
        let (mut mincode, mut maxcode) = ([0; 33], [0; 33]);
        for length in 1..=32 {
            let min = u32_at(huff, off2 + (length - 1) * 8).ok_or_else(damaged)?;
            let max = u32_at(huff, off2 + (length - 1) * 8 + 4).ok_or_else(damaged)?;
            mincode[length] = u64::from(min) << (32 - length);
            maxcode[length] = ((u64::from(max) + 1) << (32 - length)) - 1;
        }
        let mut phrases = Vec::new();
        for index in 1..count as usize {
            let cdic = records.get(first + index).filter(|cdic| cdic.starts_with(b"CDIC\0\0\0\x10")).ok_or_else(damaged)?;
            let (total, bits) = (u32_at(cdic, 8).ok_or_else(damaged)? as usize, u32_at(cdic, 12).ok_or_else(damaged)?);
            let size = 1usize.checked_shl(bits).unwrap_or(usize::MAX).min(total.saturating_sub(phrases.len()));
            for entry in 0..size {
                let offset = usize::from(u16_at(cdic, 16 + entry * 2).ok_or_else(damaged)?);
                let length = u16_at(cdic, 16 + offset).ok_or_else(damaged)?;
                let bytes = cdic.get(18 + offset..18 + offset + usize::from(length & 0x7FFF)).ok_or_else(damaged)?.to_vec();
                phrases.push(if length & 0x8000 != 0 { Phrase::Literal(bytes) } else { Phrase::Packed(bytes) });
            }
        }
        Ok(Huff { dict1, mincode, maxcode, phrases })
    }

    fn unpack(&mut self, data: &[u8], out: &mut Vec<u8>, room: u64, depth: usize) -> Result<(), Unpack> {
        if depth > MAX_PHRASE_DEPTH { return Err(Unpack::Damaged); }
        let start = out.len();
        let word = |position: usize| (0..8).fold(0u64, |word, index| word << 8 | u64::from(data.get(position + index).copied().unwrap_or(0)));
        let mut left = data.len() as i64 * 8;
        let mut position = 0;
        let mut window = word(position);
        let mut bits: i64 = 32;
        loop {
            if bits <= 0 {
                position += 4;
                window = word(position);
                bits += 32;
            }
            let code = (window >> bits) & 0xFFFF_FFFF;
            let (mut length, terminal, mut max) = self.dict1[(code >> 24) as usize];
            if !terminal {
                while code < self.mincode[length as usize] {
                    length += 1;
                    if length > 32 { return Err(Unpack::Damaged); }
                }
                max = self.maxcode[length as usize];
            }
            bits -= i64::from(length);
            left -= i64::from(length);
            if left < 0 { break; }
            let index = (max.checked_sub(code).ok_or(Unpack::Damaged)? >> (32 - length)) as usize;
            let phrase = std::mem::replace(self.phrases.get_mut(index).ok_or(Unpack::Damaged)?, Phrase::Unpacking);
            let bytes = match phrase {
                Phrase::Literal(bytes) => bytes,
                Phrase::Packed(packed) => {
                    let mut bytes = Vec::new();
                    self.unpack(&packed, &mut bytes, room, depth + 1)?;
                    bytes
                }
                Phrase::Unpacking => return Err(Unpack::Damaged),
            };
            out.extend_from_slice(&bytes);
            self.phrases[index] = Phrase::Literal(bytes);
            if (out.len() - start) as u64 > room { return Err(Unpack::TooBig); }
        }
        Ok(())
    }
}

/// Every place `needle` starts, matched without regard to ASCII case.
fn find_all(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    haystack.windows(needle.len()).enumerate().filter(|(_, window)| window.eq_ignore_ascii_case(needle)).map(|(index, _)| index).collect()
}

/// Where the guide says the table of contents starts, as a byte offset into the text.
fn guide_toc(text: &[u8], decode: &dyn Fn(&[u8]) -> String) -> Option<u64> {
    let start = *find_all(text, b"<guide").first()?;
    let end = find_all(&text[start..], b"</guide>").first().map(|end| start + end + 8).unwrap_or(text.len());
    let document = Document::from(decode(&text[start..end]).as_str());
    let reference = document.select("reference").nodes().iter().find(|node| node.attr("type").is_some_and(|kind| kind.eq_ignore_ascii_case("toc"))).cloned()?;
    reference.attr("filepos")?.trim().parse().ok()
}

/// The table of contents' `filepos` links, in order, as chapter-level bookmarks.
fn navigation(markup: &str, chapter_at: &dyn Fn(u64) -> usize) -> Vec<Entry> {
    let document = Document::from(markup);
    document.select("a[filepos]").nodes().iter().filter_map(|link| {
        let offset = link.attr("filepos")?.trim().parse::<u64>().ok()?;
        let title = tidy(&link.text());
        (!title.is_empty()).then(|| Entry { level: 1, title, target: part(chapter_at(offset)) })
    }).take(MAX_NAV_ENTRIES).collect()
}

/// Image records, named `record/{recindex}`; `recindex` counts from 1 at the first image record.
struct Images<'a> { records: &'a [&'a [u8]], first: u32 }

impl Resources for Images<'_> {
    fn read(&mut self, href: &str, _base: Option<&Path>) -> Option<(PathBuf, Vec<u8>)> {
        let index = href.strip_prefix(IMAGE_PREFIX)?.parse::<u32>().ok()?.checked_sub(1)?;
        if self.first == NONE { return None; }
        let record = self.records.get(self.first.checked_add(index)? as usize)?;
        Some((PathBuf::from(href), record.to_vec()))
    }
}

/// MOBI books built in code for tests.
#[cfg(test)]
pub mod fixtures {
    /// How a fixture book is written.
    #[derive(Clone)]
    pub struct Options { pub compression: u16, pub encryption: u16, pub version: u32, pub utf8: bool, pub multibyte: bool }

    impl Default for Options {
        fn default() -> Self { Options { compression: super::PALMDOC, encryption: 0, version: 6, utf8: true, multibyte: true } }
    }

    /// A book with `markup` as its text, `title` in the EXTH metadata and `images` after the text records.
    pub fn book(title: &str, markup: impl AsRef<[u8]>, images: &[Vec<u8>], options: &Options) -> Vec<u8> {
        let text = markup.as_ref();
        let mut records: Vec<Vec<u8>> = text.chunks(4096).map(|chunk| {
            let mut record = if options.compression == super::PALMDOC { compress(chunk) } else { chunk.to_vec() };
            if options.multibyte { record.push(0); }
            record
        }).collect();
        let text_records = records.len();
        records.extend(images.iter().cloned());
        let first_image = if images.is_empty() { super::NONE } else { text_records as u32 + 1 };
        let record0 = header(title, text.len() as u32, text_records as u16, first_image, options);
        palm_db(std::iter::once(record0).chain(records).collect())
    }

    /// Record 0: the PalmDOC and MOBI headers, then EXTH with the title, then the full name.
    pub fn header(title: &str, text_length: u32, text_records: u16, first_image: u32, options: &Options) -> Vec<u8> {
        let mut record = vec![0u8; 16 + 0xE8];
        record[0..2].copy_from_slice(&options.compression.to_be_bytes());
        record[4..8].copy_from_slice(&text_length.to_be_bytes());
        record[8..10].copy_from_slice(&text_records.to_be_bytes());
        record[10..12].copy_from_slice(&4096u16.to_be_bytes());
        record[12..14].copy_from_slice(&options.encryption.to_be_bytes());
        record[16..20].copy_from_slice(b"MOBI");
        record[20..24].copy_from_slice(&0xE8u32.to_be_bytes());
        record[24..28].copy_from_slice(&2u32.to_be_bytes());
        record[28..32].copy_from_slice(&(if options.utf8 { 65_001u32 } else { 1_252 }).to_be_bytes());
        record[36..40].copy_from_slice(&options.version.to_be_bytes());
        record[40..80].fill(0xFF);
        record[80..84].copy_from_slice(&(u32::from(text_records) + 1).to_be_bytes());
        record[108..112].copy_from_slice(&first_image.to_be_bytes());
        record[128..132].copy_from_slice(&0x40u32.to_be_bytes());
        record[168..172].copy_from_slice(&super::NONE.to_be_bytes());
        record[0xF2..0xF4].copy_from_slice(&u16::from(options.multibyte).to_be_bytes());
        let mut exth = b"EXTH".to_vec();
        let entry = [503u32.to_be_bytes().as_slice(), &(title.len() as u32 + 8).to_be_bytes(), title.as_bytes()].concat();
        exth.extend_from_slice(&(12 + entry.len() as u32).to_be_bytes());
        exth.extend_from_slice(&1u32.to_be_bytes());
        exth.extend_from_slice(&entry);
        while exth.len() % 4 != 0 { exth.push(0); }
        record.extend_from_slice(&exth);
        let name = b"Full Name";
        let at = record.len() as u32;
        record[84..88].copy_from_slice(&at.to_be_bytes());
        record[88..92].copy_from_slice(&(name.len() as u32).to_be_bytes());
        record.extend_from_slice(name);
        record.extend_from_slice(&[0, 0]);
        record
    }

    /// The PalmDB container around `records`.
    pub fn palm_db(records: Vec<Vec<u8>>) -> Vec<u8> {
        let mut bytes = vec![0u8; 78];
        bytes[..8].copy_from_slice(b"fixture\0");
        bytes[60..68].copy_from_slice(b"BOOKMOBI");
        bytes[76..78].copy_from_slice(&(records.len() as u16).to_be_bytes());
        let mut offset = 78 + records.len() * 8 + 2;
        for (index, record) in records.iter().enumerate() {
            bytes.extend_from_slice(&(offset as u32).to_be_bytes());
            bytes.extend_from_slice(&((index as u32) * 2).to_be_bytes());
            offset += record.len();
        }
        bytes.extend_from_slice(&[0, 0]);
        for record in records { bytes.extend_from_slice(&record); }
        bytes
    }

    /// PalmDOC without back references: literals, space pairs, and raw runs for everything else.
    pub fn compress(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut index = 0;
        while index < data.len() {
            let byte = data[index];
            if byte == b' ' && data.get(index + 1).is_some_and(|next| (0x40..=0x7F).contains(next)) {
                out.push(data[index + 1] ^ 0x80);
                index += 2;
            } else if byte == 0 || (0x09..=0x7F).contains(&byte) {
                out.push(byte);
                index += 1;
            } else {
                let run = data[index..].iter().take(8).take_while(|byte| !(**byte == 0 || (0x09..=0x7F).contains(*byte))).count();
                out.push(run as u8);
                out.extend_from_slice(&data[index..index + run]);
                index += run;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::contracts::ErrorKind;
    use crate::kit::pdf::layout::{Block, Span};

    fn exceeded() -> ToolError { ToolError::limit_exceeded(LimitKind::Decompression, "too big") }

    fn spans(blocks: &[Block]) -> Vec<Span> {
        let mut found = Vec::new();
        let mut blocks = blocks.to_vec();
        epub::visit(&mut blocks, &mut |_| {}, &mut |span| found.push(span.clone()));
        found
    }

    fn text_of(book: &Book) -> String { book.chapters.iter().flat_map(|chapter| spans(&chapter.blocks)).map(|span| span.text).collect() }

    #[test]
    fn palmdoc_reads_literals_runs_back_references_and_space_pairs() {
        let mut out = Vec::new();
        // "abc", then a copy of 3 bytes from 3 back ("abc"), then " x", then the raw byte 0xE9.
        let pair = (3u16 << 3) | 0x8000;
        let data = [b"abc".as_slice(), &pair.to_be_bytes(), &[b'x' ^ 0x80], &[1, 0xE9]].concat();
        palmdoc(&data, &mut out, 1_000).unwrap();
        assert_eq!(out, b"abcabc x\xE9");
        assert_eq!(palmdoc(&[0x80, 0x09], &mut Vec::new(), 1_000), Err(Unpack::Damaged), "a reference before any text is damage");
        assert_eq!(palmdoc(b"abcdef", &mut Vec::new(), 3), Err(Unpack::TooBig));
        let mut round = Vec::new();
        palmdoc(&fixtures::compress("Café Привет and more".as_bytes()), &mut round, 1_000).unwrap();
        assert_eq!(String::from_utf8(round).unwrap(), "Café Привет and more");
    }

    #[test]
    fn trailing_entries_are_stripped_from_the_end_of_each_record() {
        // One size-encoded entry (3 bytes, its size in the last byte) and one multibyte byte before it.
        assert_eq!(trailing(b"text\x00\x01\x01\x83", 0b11), 4);
        assert_eq!(trailing(b"text\x00", 0b1), 1);
        assert_eq!(trailing(b"text", 0), 0);
        assert_eq!(trailing(b"\xFF", 0b10), 1, "an entry can't claim more than the record");
    }

    /// A HUFF table where every code is 8 bits and byte `b` names phrase `255 - b`, with phrase 255 - b
    /// being the literal byte `b`, except phrase 0 (byte 0xFF) which is the compressed form of "ab".
    fn huff_book(text: &[u8]) -> Vec<u8> {
        let mut huff = b"HUFF\0\0\0\x18".to_vec();
        huff.extend_from_slice(&24u32.to_be_bytes());
        huff.extend_from_slice(&(24 + 1024u32).to_be_bytes());
        huff.extend_from_slice(&[0; 8]);
        for _ in 0..256 { huff.extend_from_slice(&((255u32 << 8) | 0x80 | 8).to_be_bytes()); }
        for _ in 0..64 { huff.extend_from_slice(&0u32.to_be_bytes()); }
        let mut entries = Vec::new();
        let mut offsets = Vec::new();
        for index in 0..256usize {
            offsets.push((256 * 2 + entries.len()) as u16);
            if index == 0 {
                entries.extend_from_slice(&2u16.to_be_bytes());
                entries.extend_from_slice(b"ab");
            } else {
                entries.extend_from_slice(&0x8001u16.to_be_bytes());
                entries.push(255 - index as u8);
            }
        }
        let mut cdic = b"CDIC\0\0\0\x10".to_vec();
        cdic.extend_from_slice(&256u32.to_be_bytes());
        cdic.extend_from_slice(&8u32.to_be_bytes());
        for offset in offsets { cdic.extend_from_slice(&offset.to_be_bytes()); }
        cdic.extend_from_slice(&entries);
        let options = fixtures::Options { compression: HUFF_CDIC, multibyte: false, ..Default::default() };
        let mut record0 = fixtures::header("Huff", (text.len() + 2) as u32, 1, NONE, &options);
        record0[112..116].copy_from_slice(&2u32.to_be_bytes());
        record0[116..120].copy_from_slice(&2u32.to_be_bytes());
        let mut packed = text.to_vec();
        packed.push(0xFF);
        fixtures::palm_db(vec![record0, packed, huff, cdic])
    }

    #[test]
    fn huff_cdic_books_decompress_with_nested_phrases() {
        let book = parse(&huff_book(b"<p>Huffman text "), 1 << 20, exceeded(), 1.).unwrap();
        assert_eq!(text_of(&book).trim(), "Huffman text ab");
        assert_eq!(book.title.as_deref(), Some("Huff"));
        assert!(matches!(parse(&huff_book(b"<p>Huffman text "), 4, exceeded(), 1.), Err(error) if matches!(error.kind, ErrorKind::LimitExceeded { .. })));
    }

    #[test]
    fn chapters_split_at_page_breaks_with_images_links_and_the_guide_toc() {
        let mut png = Vec::new();
        image::DynamicImage::new_rgb8(4, 4).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let head = r#"<html><head><guide><reference type="toc" title="Contents" filepos=TOC0000000 /></guide></head><body>"#;
        let toc = r#"<p>Contents</p><p><a filepos=ONE0000000>Chapter One</a></p><p><a filepos=TWO0000000>Chapter Two</a></p>"#;
        let one = r#"<mbp:pagebreak/><h1>Chapter One</h1><p>First words <img recindex="00001"> <a href="https://example.com">web</a> <a filepos=TWO0000000>next</a></p>"#;
        let two = r#"<mbp:pagebreak/><h1>Chapter Two</h1><p>Second words</p></body></html>"#;
        let (toc_at, one_at) = (head.len(), head.len() + toc.len());
        let two_at = one_at + one.len();
        let markup = [head, toc, one, two].concat()
            .replace("TOC0000000", &format!("{toc_at:010}"))
            .replace("ONE0000000", &format!("{one_at:010}"))
            .replace("TWO0000000", &format!("{two_at:010}"));
        let bytes = fixtures::book("Linked", &markup, &[png], &fixtures::Options::default());
        let book = parse(&bytes, 1 << 20, exceeded(), 1.).unwrap();
        assert_eq!(book.chapters.len(), 3);
        assert_eq!(book.pictures.len(), 1);
        let mut images = Vec::new();
        epub::visit(&mut book.chapters[1].blocks.clone(), &mut |block| if let Block::Image(index) = block { images.push(*index) }, &mut |_| {});
        assert_eq!(images, [0]);
        let links: Vec<String> = book.chapters.iter().flat_map(|chapter| spans(&chapter.blocks)).filter_map(|span| span.link).collect();
        assert!(links.contains(&"https://example.com".to_string()));
        assert!(links.contains(&"#part-2".to_string()), "{links:?}");
        let navigation: Vec<(String, String)> = book.navigation.iter().map(|entry| (entry.title.clone(), entry.target.clone())).collect();
        assert_eq!(navigation, [("Chapter One".to_string(), "part-1".to_string()), ("Chapter Two".to_string(), "part-2".to_string())]);
    }

    #[test]
    fn windows_1252_books_and_uncompressed_records_read() {
        let options = fixtures::Options { compression: NO_COMPRESSION, utf8: false, multibyte: false, ..Default::default() };
        let bytes = fixtures::book("Old", b"<p>Caf\xE9</p>", &[], &options);
        let book = parse(&bytes, 1 << 20, exceeded(), 1.).unwrap();
        assert_eq!(text_of(&book).trim(), "Café");
    }

    #[test]
    fn protected_kf8_only_and_damaged_books_are_refused_with_their_reason() {
        let protected = fixtures::book("Locked", "<p>Secret</p>", &[], &fixtures::Options { encryption: 2, ..Default::default() });
        assert!(matches!(parse(&protected, 1 << 20, exceeded(), 1.), Err(error) if error.kind == ErrorKind::InvalidInput && error.message == PROTECTED));
        let kf8 = fixtures::book("New", "<p>New</p>", &[], &fixtures::Options { version: 8, ..Default::default() });
        assert!(matches!(parse(&kf8, 1 << 20, exceeded(), 1.), Err(error) if error.kind == ErrorKind::InvalidInput && error.message == KF8_ONLY));
        assert!(matches!(parse(b"not a book", 1 << 20, exceeded(), 1.), Err(error) if error.kind == ErrorKind::InvalidInput && error.message == DAMAGED));
        let mut truncated = fixtures::book("Short", "<p>Short</p>", &[], &fixtures::Options::default());
        truncated[78..82].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(matches!(parse(&truncated, 1 << 20, exceeded(), 1.), Err(error) if error.kind == ErrorKind::InvalidInput && error.message == DAMAGED));
        let big = fixtures::book("Big", &"<p>word</p>".repeat(100), &[], &fixtures::Options::default());
        assert!(matches!(parse(&big, 100, exceeded(), 1.), Err(error) if matches!(error.kind, ErrorKind::LimitExceeded { .. })));
    }
}
