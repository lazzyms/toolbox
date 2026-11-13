//! The compound-file reader shared by the Word 97-2003 (DOC), Excel 97-2003 (XLS) and PowerPoint 97-2003 (PPT) readers.

use std::io::{Cursor, Read};

use cfb::CompoundFile;
use encoding_rs::{Encoding, UTF_16LE, UTF_8, WINDOWS_1252};

use crate::kit::contracts::ToolError;

const SUMMARY: &str = "/\u{5}SummaryInformation";
const TITLE: u32 = 2;
const AUTHOR: u32 = 4;
const CODEPAGE: u32 = 1;
const VT_I2: u32 = 0x02;
const VT_LPSTR: u32 = 0x1E;
const VT_LPWSTR: u32 = 0x1F;

/// A compound file whose streams share one expansion budget; going over returns `exceeded`.
pub(super) struct Compound<'a> { file: CompoundFile<Cursor<&'a [u8]>>, budget: u64, exceeded: ToolError, damaged: &'static str }

impl<'a> Compound<'a> {
    pub(super) fn open(bytes: &'a [u8], budget: u64, exceeded: ToolError, damaged: &'static str) -> Result<Self, ToolError> {
        let file = CompoundFile::open(Cursor::new(bytes)).map_err(|_| ToolError::invalid_input(damaged))?;
        Ok(Self { file, budget, exceeded, damaged })
    }

    pub(super) fn damaged(&self) -> ToolError { ToolError::invalid_input(self.damaged) }

    /// The named stream, or `None` when the file has no such stream. The stream's stated size is checked before it is read.
    pub(super) fn stream(&mut self, name: &str) -> Result<Option<Vec<u8>>, ToolError> {
        if !self.file.is_stream(name) { return Ok(None); }
        let length = self.file.entry(name).map_err(|_| self.damaged())?.len();
        if length > self.budget { return Err(self.exceeded.clone()); }
        let damaged = self.damaged;
        let mut data = Vec::new();
        let stream = self.file.open_stream(name).map_err(|_| ToolError::invalid_input(damaged))?;
        stream.take(length).read_to_end(&mut data).map_err(|_| ToolError::invalid_input(damaged))?;
        self.budget -= data.len() as u64;
        Ok(Some(data))
    }

    /// The title and author recorded in the file's summary information, when it has them.
    pub(super) fn summary(&mut self) -> Result<(Option<String>, Option<String>), ToolError> {
        Ok(self.stream(SUMMARY)?.map(|bytes| summary(&bytes)).unwrap_or_default())
    }
}

pub(super) fn u16_at(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(offset..offset.checked_add(2)?)?.try_into().ok()?))
}

pub(super) fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?))
}

pub(super) fn i16_at(bytes: &[u8], offset: usize) -> Option<i16> { u16_at(bytes, offset).map(|value| value as i16) }

pub(super) fn i32_at(bytes: &[u8], offset: usize) -> Option<i32> { u32_at(bytes, offset).map(|value| value as i32) }

pub(super) fn f64_at(bytes: &[u8], offset: usize) -> Option<f64> {
    Some(f64::from_le_bytes(bytes.get(offset..offset.checked_add(8)?)?.try_into().ok()?))
}

/// UTF-16LE text, ending at the first NUL.
pub(super) fn utf16(bytes: &[u8]) -> String {
    let units = bytes.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).take_while(|unit| *unit != 0).collect::<Vec<_>>();
    String::from_utf16_lossy(&units)
}

/// The text encoding for a Windows code page number; unknown code pages read as Windows-1252.
pub(super) fn codepage(number: u16) -> &'static Encoding {
    let label = match number {
        1200 => return UTF_16LE,
        65001 => return UTF_8,
        874 | 1250..=1258 => format!("windows-{number}"),
        932 => "shift_jis".into(),
        936 => "gbk".into(),
        949 => "euc-kr".into(),
        950 => "big5".into(),
        10000 => "macintosh".into(),
        _ => return WINDOWS_1252,
    };
    Encoding::for_label(label.as_bytes()).unwrap_or(WINDOWS_1252)
}

/// Title and author from a SummaryInformation property set.
fn summary(bytes: &[u8]) -> (Option<String>, Option<String>) {
    let Some(section) = u32_at(bytes, 44).map(|offset| offset as usize) else { return (None, None) };
    let Some(count) = u32_at(bytes, section + 4) else { return (None, None) };
    let properties = (0..count.min(1024) as usize)
        .map_while(|index| Some((u32_at(bytes, section + 8 + index * 8)?, section + u32_at(bytes, section + 12 + index * 8)? as usize)))
        .collect::<Vec<_>>();
    let encoding = properties.iter().find(|(id, _)| *id == CODEPAGE)
        .filter(|(_, offset)| u32_at(bytes, *offset) == Some(VT_I2))
        .and_then(|(_, offset)| u16_at(bytes, offset + 4))
        .map_or(WINDOWS_1252, codepage);
    let text = |id: u32| -> Option<String> {
        let offset = properties.iter().find(|(property, _)| *property == id)?.1;
        let length = u32_at(bytes, offset + 4)? as usize;
        let value = match u32_at(bytes, offset)? {
            VT_LPSTR if encoding == UTF_16LE => utf16(bytes.get(offset + 8..offset + 8 + length.checked_mul(2)?)?),
            VT_LPSTR => {
                let raw = bytes.get(offset + 8..offset + 8 + length)?;
                let raw = &raw[..raw.iter().position(|byte| *byte == 0).unwrap_or(raw.len())];
                encoding.decode_without_bom_handling(raw).0.into_owned()
            }
            VT_LPWSTR => utf16(bytes.get(offset + 8..offset + 8 + length.checked_mul(2)?)?),
            _ => return None,
        };
        let value = value.trim().to_string();
        (!value.is_empty()).then_some(value)
    };
    (text(TITLE), text(AUTHOR))
}

#[cfg(test)]
pub(crate) mod fixtures {
    /// A SummaryInformation stream with a Windows-1252 title and author.
    pub(crate) fn summary(title: &str, author: &str) -> Vec<u8> {
        let mut values = Vec::new();
        let mut entries = Vec::new();
        let header = 8 + 3 * 8;
        let mut property = |id: u32, value: Vec<u8>| {
            entries.push((id, (header + values.len()) as u32));
            values.extend(value);
            while values.len() % 4 != 0 { values.push(0); }
        };
        property(1, [2u32.to_le_bytes().to_vec(), 1252u16.to_le_bytes().to_vec(), vec![0, 0]].concat());
        for (id, text) in [(2u32, title), (4, author)] {
            let mut bytes = encoding_rs::WINDOWS_1252.encode(text).0.into_owned();
            bytes.push(0);
            property(id, [0x1Eu32.to_le_bytes().to_vec(), (bytes.len() as u32).to_le_bytes().to_vec(), bytes].concat());
        }
        let mut section = ((header + values.len()) as u32).to_le_bytes().to_vec();
        section.extend((entries.len() as u32).to_le_bytes());
        for (id, offset) in entries { section.extend(id.to_le_bytes()); section.extend(offset.to_le_bytes()); }
        section.extend(values);
        let mut stream = vec![0xFE, 0xFF, 0, 0, 0, 0, 0, 0];
        stream.extend([0u8; 16]);
        stream.extend(1u32.to_le_bytes());
        stream.extend([0u8; 16]);
        stream.extend(48u32.to_le_bytes());
        stream.extend(section);
        stream
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::office::tests::cfb_with_streams;

    #[test]
    fn streams_share_one_budget_and_the_summary_gives_title_and_author() {
        let bytes = cfb_with_streams(&[("/One", &[1; 600]), ("/Two", &[2; 600]), (SUMMARY, &fixtures::summary("Caf\u{e9} plan", "Ana"))]);
        let exceeded = ToolError::invalid_input("too big");
        let mut file = Compound::open(&bytes, 1000, exceeded.clone(), "damaged").unwrap();
        assert_eq!(file.stream("/One").unwrap().unwrap().len(), 600);
        assert_eq!(file.stream("/Missing").unwrap(), None);
        assert_eq!(file.stream("/Two").unwrap_err().message, "too big");

        let mut file = Compound::open(&bytes, 10_000, exceeded, "damaged").unwrap();
        assert_eq!(file.summary().unwrap(), (Some("Caf\u{e9} plan".into()), Some("Ana".into())));
        assert_eq!(Compound::open(b"not a compound file", 10, ToolError::invalid_input("x"), "damaged").err().unwrap().message, "damaged");
    }

    #[test]
    fn code_pages_map_to_encodings() {
        assert_eq!(codepage(1251).name(), "windows-1251");
        assert_eq!(codepage(932).name(), "Shift_JIS");
        assert_eq!(codepage(1).name(), "windows-1252");
        assert_eq!(utf16(&[0x41, 0, 0x42, 0, 0, 0, 0x43, 0]), "AB");
    }
}
