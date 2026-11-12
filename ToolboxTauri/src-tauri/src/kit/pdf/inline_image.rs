use lopdf::content::Content;
use lopdf::{Dictionary, Object, Stream};

/// The inline images (BI … ID … EI) of one content stream, in content order, as image streams with their
/// abbreviated keys written out in full. A colour space named by the resources is looked up with `named_space`.
/// An image whose dictionary can't be read is kept with an empty dictionary, so it is reported as damaged.
pub(crate) fn inline_images(content: &[u8], named_space: impl Fn(&[u8]) -> Option<Object>) -> Vec<Stream> {
    let mut images = Vec::new();
    let mut at = 0;
    while let Some((start, end)) = next_keyword(content, at) {
        at = end;
        if &content[start..end] != b"BI" { continue; }
        let Some((id_start, id_end)) = keyword_after(content, end, b"ID") else { break };
        let dict = parse_dictionary(&content[end..id_start]).map(|dict| expand(dict, &named_space)).unwrap_or_default();
        let data_start = (id_end + 1).min(content.len());
        let Some((data_end, after)) = data_end(content, data_start, &dict) else { break };
        images.push(Stream::new(dict, content[data_start..data_end].to_vec()));
        at = after;
    }
    images
}

fn is_white(byte: u8) -> bool {
    matches!(byte, 0 | b'\t' | b'\n' | 0x0c | b'\r' | b' ')
}

fn is_delimiter(byte: u8) -> bool {
    matches!(byte, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

fn is_regular(byte: u8) -> bool {
    !is_white(byte) && !is_delimiter(byte)
}

/// The next bare keyword or number at or after `at`, skipping comments, strings and names.
fn next_keyword(data: &[u8], mut at: usize) -> Option<(usize, usize)> {
    while let Some(&byte) = data.get(at) {
        match byte {
            b'%' => while data.get(at).is_some_and(|byte| !matches!(byte, b'\r' | b'\n')) { at += 1 },
            b'(' => at = after_literal_string(data, at),
            b'<' if data.get(at + 1) == Some(&b'<') => at += 2,
            b'<' => at = data[at..].iter().position(|&byte| byte == b'>').map_or(data.len(), |end| at + end + 1),
            b'/' => {
                at += 1;
                while data.get(at).copied().is_some_and(is_regular) { at += 1 }
            }
            byte if is_white(byte) || is_delimiter(byte) => at += 1,
            _ => {
                let start = at;
                while data.get(at).copied().is_some_and(is_regular) { at += 1 }
                return Some((start, at));
            }
        }
    }
    None
}

fn after_literal_string(data: &[u8], start: usize) -> usize {
    let mut depth = 0usize;
    let mut at = start;
    while let Some(&byte) = data.get(at) {
        match byte {
            b'\\' => at += 1,
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 { return at + 1; }
            }
            _ => {}
        }
        at += 1;
    }
    data.len()
}

fn keyword_after(data: &[u8], mut at: usize, keyword: &[u8]) -> Option<(usize, usize)> {
    while let Some((start, end)) = next_keyword(data, at) {
        if &data[start..end] == keyword { return Some((start, end)); }
        at = end;
    }
    None
}

fn parse_dictionary(entries: &[u8]) -> Option<Dictionary> {
    let source = [b"<<".as_slice(), entries, b">> BI\n"].concat();
    let content = Content::decode(&source).ok()?;
    match content.operations.first()?.operands.first()? {
        Object::Dictionary(dict) => Some(dict.clone()),
        _ => None,
    }
}

fn full_key(key: &[u8]) -> &[u8] {
    match key {
        b"W" => b"Width",
        b"H" => b"Height",
        b"BPC" => b"BitsPerComponent",
        b"CS" => b"ColorSpace",
        b"D" => b"Decode",
        b"DP" => b"DecodeParms",
        b"F" => b"Filter",
        b"IM" => b"ImageMask",
        b"I" => b"Interpolate",
        b"L" => b"Length",
        other => other,
    }
}

const DEVICE_SPACES: [&[u8]; 6] = [b"G", b"RGB", b"CMYK", b"DeviceGray", b"DeviceRGB", b"DeviceCMYK"];

fn expand(dict: Dictionary, named_space: impl Fn(&[u8]) -> Option<Object>) -> Dictionary {
    let mut expanded = Dictionary::new();
    for (key, value) in dict {
        let key = full_key(&key).to_vec();
        let value = match value {
            Object::Name(name) if key == b"ColorSpace" && !DEVICE_SPACES.contains(&name.as_slice()) => named_space(&name).unwrap_or(Object::Name(name)),
            value => value,
        };
        expanded.set(key, value);
    }
    expanded
}

/// Where the image data ends and where scanning resumes after EI. A stated length, or the exact size of
/// unfiltered samples, is used when EI follows it; otherwise the data runs to the first EI after whitespace.
fn data_end(data: &[u8], start: usize, dict: &Dictionary) -> Option<(usize, usize)> {
    let ends_at = |at: usize| at == data.len() || !is_regular(data[at]);
    if let Some(end) = known_length(dict).and_then(|length| start.checked_add(length)).filter(|end| *end <= data.len()) {
        let mut at = end;
        while data.get(at).copied().is_some_and(is_white) { at += 1 }
        if data[at..].starts_with(b"EI") && ends_at(at + 2) { return Some((end, at + 2)); }
    }
    (start + 1..data.len().saturating_sub(1))
        .find(|&at| is_white(data[at - 1]) && data[at..].starts_with(b"EI") && ends_at(at + 2))
        .map(|at| (at - 1, at + 2))
}

fn known_length(dict: &Dictionary) -> Option<usize> {
    if let Ok(length) = dict.get(b"Length").and_then(Object::as_i64) { return usize::try_from(length).ok(); }
    if dict.has(b"Filter") { return None; }
    let number = |key: &[u8]| dict.get(key).and_then(Object::as_i64).ok().and_then(|value| usize::try_from(value).ok());
    let mask = dict.get(b"ImageMask").and_then(Object::as_bool).unwrap_or(false);
    let (bits, components) = if mask { (1, 1) } else {
        let components = match dict.get(b"ColorSpace").ok()? {
            Object::Name(name) => match name.as_slice() {
                b"G" | b"DeviceGray" | b"CalGray" => 1,
                b"RGB" | b"DeviceRGB" | b"CalRGB" => 3,
                b"CMYK" | b"DeviceCMYK" => 4,
                _ => return None,
            },
            Object::Array(items) if matches!(items.first().and_then(|item| item.as_name().ok()), Some(b"I" | b"Indexed")) => 1,
            _ => return None,
        };
        (number(b"BitsPerComponent")?, components)
    };
    let row = number(b"Width")?.checked_mul(bits)?.checked_mul(components)?.div_ceil(8);
    row.checked_mul(number(b"Height")?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_spaces(_: &[u8]) -> Option<Object> { None }

    #[test]
    fn abbreviated_keys_are_written_out_and_data_is_cut_at_its_exact_length() {
        let content = b"q 2 0 0 2 0 0 cm\nBI /W 2 /H 1 /BPC 8 /CS /RGB ID \x01 EI \x06\nEI Q";
        let images = inline_images(content, no_spaces);
        assert_eq!(images.len(), 1);
        let dict = &images[0].dict;
        assert_eq!(dict.get(b"Width").unwrap().as_i64().unwrap(), 2);
        assert_eq!(dict.get(b"BitsPerComponent").unwrap().as_i64().unwrap(), 8);
        assert_eq!(dict.get(b"ColorSpace").unwrap().as_name().unwrap(), b"RGB");
        assert_eq!(images[0].content, b"\x01 EI \x06");
    }

    #[test]
    fn filtered_data_runs_to_the_first_ei_after_whitespace_and_named_spaces_are_resolved() {
        let content = b"(BI in a string) Tj % BI in a comment\nBI /W 1 /H 1 /BPC 8 /CS /Cs1 /F /AHx ID 7f> EI\nBI /W 1 /H 1 /IM true ID \x80 EI";
        let images = inline_images(content, |name| (name == b"Cs1").then(|| Object::Name(b"DeviceGray".to_vec())));
        assert_eq!(images.len(), 2);
        assert_eq!(images[0].dict.get(b"Filter").unwrap().as_name().unwrap(), b"AHx");
        assert_eq!(images[0].dict.get(b"ColorSpace").unwrap().as_name().unwrap(), b"DeviceGray");
        assert_eq!(images[0].content, b"7f>");
        assert!(images[1].dict.get(b"ImageMask").unwrap().as_bool().unwrap());
        assert_eq!(images[1].content, b"\x80");
    }
}
