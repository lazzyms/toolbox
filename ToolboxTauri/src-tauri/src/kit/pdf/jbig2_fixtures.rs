//! JBIG2 fixtures generated in code, for the decoder spike (#623) and the
//! production JBIG2 coverage (#630).
//!
//! Each fixture is a stream in the embedded organisation (ITU-T T.88 Annex D.3)
//! that a PDF `JBIG2Decode` filter carries: segment headers and data, with no
//! file header and no end-of-page or end-of-file segment. A small MQ
//! arithmetic encoder (T.88 Annex E.2) writes the region data, so every
//! fixture's pixels are known exactly. The cases are:
//!
//! - a generic region (arithmetic, template 0);
//! - a text region whose symbol dictionary is in the same page stream;
//! - the same text region with its symbol dictionary in a separate global
//!   stream, as a PDF `/JBIG2Globals` stream supplies it.
//!
//! Pixels are row-major with `true` for black, which is JBIG2's 1 bit. A PDF
//! `JBIG2Decode` filter returns the inverse (0 for black) for a `DeviceGray`
//! image, so a PDF consumer must invert before writing grey pixels.

use lopdf::{dictionary, Document, Object, Stream};

pub(crate) struct Jbig2Fixture {
    pub(crate) name: &'static str,
    /// The page's segments, as the content of a `JBIG2Decode` image stream.
    pub(crate) stream: Vec<u8>,
    /// Global segments, as the content of a `/JBIG2Globals` stream.
    pub(crate) globals: Option<Vec<u8>>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Row-major; `true` is black.
    pub(crate) black: Vec<bool>,
}

pub(crate) fn all() -> Vec<Jbig2Fixture> {
    vec![generic_region(), text_region_with_symbol_dictionary(), text_region_with_embedded_globals()]
}

/// A 23×9 page holding one immediate generic region.
pub(crate) fn generic_region() -> Jbig2Fixture {
    let (width, height) = (23, 9);
    let mut bitmap = Bitmap::new(width, height);
    for y in 0..height {
        for x in 0..width {
            bitmap.set(x, y, x == y || (x * 7 + y * 3) % 5 == 0 || (x >= 18 && y >= 6));
        }
    }

    let mut encoder = MqEncoder::new();
    let mut contexts = vec![Context::default(); 1 << 16];
    encode_generic(&mut encoder, &mut contexts, &bitmap);

    let mut data = region_info(width, height, 0, 0);
    data.push(0); // MMR off, template 0, TPGDON off.
    data.extend(DEFAULT_TEMPLATE0_AT.iter().map(|&value| value as u8));
    data.extend(encoder.finish());

    let mut stream = Vec::new();
    write_segment(&mut stream, 0, PAGE_INFORMATION, &[], 1, &page_information(width, height));
    write_segment(&mut stream, 1, IMMEDIATE_GENERIC_REGION, &[], 1, &data);

    Jbig2Fixture { name: "generic region", stream, globals: None, width, height, black: bitmap.pixels }
}

/// A text region that refers to a symbol dictionary in the same page stream.
pub(crate) fn text_region_with_symbol_dictionary() -> Jbig2Fixture {
    let text = TextPage::new();
    let mut stream = Vec::new();
    write_segment(&mut stream, 0, PAGE_INFORMATION, &[], 1, &page_information(text.width, text.height));
    write_segment(&mut stream, 1, SYMBOL_DICTIONARY, &[], 1, &symbol_dictionary(&text.symbols));
    write_segment(&mut stream, 2, IMMEDIATE_TEXT_REGION, &[1], 1, &text.region_data());

    Jbig2Fixture {
        name: "text region with symbol dictionary",
        stream,
        globals: None,
        width: text.width,
        height: text.height,
        black: text.expected().pixels,
    }
}

/// The same text region, with the symbol dictionary moved to a global stream
/// (page association 0) that the page stream refers to.
pub(crate) fn text_region_with_embedded_globals() -> Jbig2Fixture {
    let text = TextPage::new();
    let mut globals = Vec::new();
    write_segment(&mut globals, 0, SYMBOL_DICTIONARY, &[], 0, &symbol_dictionary(&text.symbols));

    let mut stream = Vec::new();
    write_segment(&mut stream, 1, PAGE_INFORMATION, &[], 1, &page_information(text.width, text.height));
    write_segment(&mut stream, 2, IMMEDIATE_TEXT_REGION, &[0], 1, &text.region_data());

    Jbig2Fixture {
        name: "text region with embedded globals",
        stream,
        globals: Some(globals),
        width: text.width,
        height: text.height,
        black: text.expected().pixels,
    }
}

/// A PDF with one page per fixture. Each page draws one `JBIG2Decode` image
/// XObject (`/Im0`); a fixture with globals gets a `/JBIG2Globals` stream in
/// the image's `/DecodeParms`. The PDF has no other images.
pub(crate) fn pdf_with_images(fixtures: &[Jbig2Fixture]) -> Document {
    let mut document = Document::with_version("1.7");
    let pages_id = document.new_object_id();
    let mut kids = Vec::new();
    for fixture in fixtures {
        let mut image = dictionary! {
            "Type" => "XObject", "Subtype" => "Image",
            "Width" => fixture.width as i64, "Height" => fixture.height as i64,
            "ColorSpace" => "DeviceGray", "BitsPerComponent" => 1, "Filter" => "JBIG2Decode",
        };
        if let Some(globals) = &fixture.globals {
            let globals_id = document.add_object(Stream::new(dictionary! {}, globals.clone()));
            image.set("DecodeParms", dictionary! { "JBIG2Globals" => globals_id });
        }
        let image_id = document.add_object(Stream::new(image, fixture.stream.clone()));
        let content = format!("q {} 0 0 {} 0 0 cm /Im0 Do Q", fixture.width, fixture.height);
        let content_id = document.add_object(Stream::new(dictionary! {}, content.into_bytes()));
        let page_id = document.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => content_id,
            "MediaBox" => vec![0.into(), 0.into(), (fixture.width as i64).into(), (fixture.height as i64).into()],
            "Resources" => dictionary! { "XObject" => dictionary! { "Im0" => image_id } },
        });
        kids.push(Object::Reference(page_id));
    }
    let count = kids.len() as i64;
    document.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }.into());
    let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    document.trailer.set("Root", catalog_id);
    document
}

// Segment types (T.88 7.3).
const SYMBOL_DICTIONARY: u8 = 0;
const IMMEDIATE_TEXT_REGION: u8 = 6;
const IMMEDIATE_GENERIC_REGION: u8 = 38;
const PAGE_INFORMATION: u8 = 48;

/// Nominal adaptive template pixels for template 0 (T.88 6.2.5.3).
const DEFAULT_TEMPLATE0_AT: [i8; 8] = [3, -1, -3, -1, 2, -2, -2, -2];

/// Text region flag word: REFCORNER = TOPLEFT, one strip, arithmetic coding,
/// no refinement, OR combination, SBDSOFFSET 0 (T.88 7.4.3.1.1).
const TEXT_REGION_FLAGS: u16 = 1 << 4;

#[derive(Clone)]
struct Bitmap {
    width: u32,
    height: u32,
    pixels: Vec<bool>,
}

impl Bitmap {
    fn new(width: u32, height: u32) -> Self {
        Self { width, height, pixels: vec![false; (width * height) as usize] }
    }

    fn from_rows(rows: &[&str]) -> Self {
        let mut bitmap = Self::new(rows[0].len() as u32, rows.len() as u32);
        for (y, row) in rows.iter().enumerate() {
            for (x, cell) in row.bytes().enumerate() {
                bitmap.set(x as u32, y as u32, cell == b'#');
            }
        }
        bitmap
    }

    fn set(&mut self, x: u32, y: u32, black: bool) {
        self.pixels[(y * self.width + x) as usize] = black;
    }

    /// The pixel at (x, y), or white outside the bitmap.
    fn bit(&self, x: i32, y: i32) -> usize {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return 0;
        }
        self.pixels[(y as u32 * self.width + x as u32) as usize] as usize
    }
}

/// Three symbols in two height classes, placed in two strips.
struct TextPage {
    width: u32,
    height: u32,
    /// Symbol bitmaps in dictionary order; heights must not decrease.
    symbols: Vec<Bitmap>,
    /// (strip top T, [(S, symbol id)]) with REFCORNER = TOPLEFT.
    strips: Vec<(i32, Vec<(i32, u32)>)>,
}

impl TextPage {
    fn new() -> Self {
        let dot = Bitmap::from_rows(&["##", "##"]);
        let ell = Bitmap::from_rows(&["#...", "#...", "#...", "#...", "####"]);
        let tee = Bitmap::from_rows(&["#####", "..#..", "..#..", "..#..", "..#.."]);
        Self {
            width: 24,
            height: 8,
            symbols: vec![dot, ell, tee],
            strips: vec![(1, vec![(1, 1), (6, 2), (15, 1)]), (5, vec![(12, 0), (20, 0)])],
        }
    }

    fn instance_count(&self) -> u32 {
        self.strips.iter().map(|(_, instances)| instances.len() as u32).sum()
    }

    fn expected(&self) -> Bitmap {
        let mut page = Bitmap::new(self.width, self.height);
        for (t, instances) in &self.strips {
            for &(s, id) in instances {
                let symbol = &self.symbols[id as usize];
                for y in 0..symbol.height {
                    for x in 0..symbol.width {
                        if symbol.bit(x as i32, y as i32) == 1 {
                            page.set(s as u32 + x, *t as u32 + y, true);
                        }
                    }
                }
            }
        }
        page
    }

    /// Text region segment data (T.88 7.4.3, decoded by 6.4.5).
    fn region_data(&self) -> Vec<u8> {
        let symbol_code_length = 32 - (self.symbols.len() as u32 - 1).leading_zeros();
        let mut encoder = MqEncoder::new();
        let (mut iadt, mut iafs, mut iads) = (IntegerContexts::new(), IntegerContexts::new(), IntegerContexts::new());
        let mut iaid = vec![Context::default(); 1 << (symbol_code_length + 1)];

        // The initial STRIPT is the negated first IADT value.
        iadt.encode(&mut encoder, Some(0));
        let (mut strip_t, mut first_s) = (0, 0);
        for (t, instances) in &self.strips {
            iadt.encode(&mut encoder, Some(t - strip_t));
            strip_t = *t;
            let mut current_s = 0;
            for (index, &(s, id)) in instances.iter().enumerate() {
                if index == 0 {
                    iafs.encode(&mut encoder, Some(s - first_s));
                    first_s = s;
                } else {
                    iads.encode(&mut encoder, Some(s - current_s));
                }
                // One strip per T value, so no IAIT value is coded.
                encode_symbol_id(&mut encoder, &mut iaid, symbol_code_length, id);
                current_s = s + self.symbols[id as usize].width as i32 - 1;
            }
            iads.encode(&mut encoder, None);
        }

        let mut data = region_info(self.width, self.height, 0, 0);
        data.extend(TEXT_REGION_FLAGS.to_be_bytes());
        data.extend(self.instance_count().to_be_bytes());
        data.extend(encoder.finish());
        data
    }
}

/// Symbol dictionary segment data exporting every symbol (T.88 7.4.2, decoded
/// by 6.5.5) with arithmetic coding, template 0 and no refinement.
fn symbol_dictionary(symbols: &[Bitmap]) -> Vec<u8> {
    let mut encoder = MqEncoder::new();
    let (mut iadh, mut iadw, mut iaex) = (IntegerContexts::new(), IntegerContexts::new(), IntegerContexts::new());
    let mut generic = vec![Context::default(); 1 << 16];

    let mut class_height = 0;
    let mut index = 0;
    while index < symbols.len() {
        let height = symbols[index].height as i32;
        iadh.encode(&mut encoder, Some(height - class_height));
        class_height = height;
        let mut symbol_width = 0;
        while index < symbols.len() && symbols[index].height as i32 == class_height {
            let symbol = &symbols[index];
            iadw.encode(&mut encoder, Some(symbol.width as i32 - symbol_width));
            symbol_width = symbol.width as i32;
            encode_generic(&mut encoder, &mut generic, symbol);
            index += 1;
        }
        iadw.encode(&mut encoder, None);
    }
    // Export flags as run lengths: no unexported run, then every new symbol.
    iaex.encode(&mut encoder, Some(0));
    iaex.encode(&mut encoder, Some(symbols.len() as i32));

    let count = symbols.len() as u32;
    let mut data = 0u16.to_be_bytes().to_vec();
    data.extend(DEFAULT_TEMPLATE0_AT.iter().map(|&value| value as u8));
    data.extend(count.to_be_bytes()); // SDNUMEXSYMS
    data.extend(count.to_be_bytes()); // SDNUMNEWSYMS
    data.extend(encoder.finish());
    data
}

/// Segment header and data (T.88 7.2) for up to four referred-to segments.
/// Referred-to segments are not retained after the referring segment.
fn write_segment(out: &mut Vec<u8>, number: u32, kind: u8, referred: &[u32], page: u8, data: &[u8]) {
    assert!(referred.len() <= 4 && number <= 256);
    out.extend(number.to_be_bytes());
    out.push(kind);
    // Retain a symbol dictionary for the region segments that refer to it.
    out.push((referred.len() as u8) << 5 | (kind == SYMBOL_DICTIONARY) as u8);
    out.extend(referred.iter().map(|&segment| segment as u8));
    out.push(page);
    out.extend((data.len() as u32).to_be_bytes());
    out.extend(data);
}

fn page_information(width: u32, height: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity(19);
    data.extend(width.to_be_bytes());
    data.extend(height.to_be_bytes());
    data.extend(0u32.to_be_bytes()); // X resolution unknown.
    data.extend(0u32.to_be_bytes()); // Y resolution unknown.
    data.push(0); // White default pixel, OR combination.
    data.extend(0u16.to_be_bytes()); // Not striped.
    data
}

fn region_info(width: u32, height: u32, x: u32, y: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity(17);
    for value in [width, height, x, y] {
        data.extend(value.to_be_bytes());
    }
    data.push(0); // OR combination with the page.
    data
}

/// Generic region encoding with template 0, nominal AT pixels and TPGDON off
/// (T.88 6.2.5.3). Bit order matches the decoder's 16-bit context.
fn encode_generic(encoder: &mut MqEncoder, contexts: &mut [Context], bitmap: &Bitmap) {
    let at = DEFAULT_TEMPLATE0_AT.map(i32::from);
    for y in 0..bitmap.height as i32 {
        for x in 0..bitmap.width as i32 {
            let p = |dx: i32, dy: i32| bitmap.bit(x + dx, y + dy);
            let context = p(-1, 0)
                | p(-2, 0) << 1
                | p(-3, 0) << 2
                | p(-4, 0) << 3
                | p(at[0], at[1]) << 4
                | p(2, -1) << 5
                | p(1, -1) << 6
                | p(0, -1) << 7
                | p(-1, -1) << 8
                | p(-2, -1) << 9
                | p(at[2], at[3]) << 10
                | p(at[4], at[5]) << 11
                | p(1, -2) << 12
                | p(0, -2) << 13
                | p(-1, -2) << 14
                | p(at[6], at[7]) << 15;
            encoder.encode(&mut contexts[context], p(0, 0) == 1);
        }
    }
}

/// Symbol ID encoding, the inverse of the IAID procedure (T.88 A.3).
fn encode_symbol_id(encoder: &mut MqEncoder, contexts: &mut [Context], code_length: u32, id: u32) {
    let mut prev = 1usize;
    for shift in (0..code_length).rev() {
        let bit = (id >> shift) & 1 == 1;
        encoder.encode(&mut contexts[prev], bit);
        prev = (prev << 1) | bit as usize;
    }
}

/// One integer arithmetic coder's contexts, the inverse of T.88 A.2.
struct IntegerContexts(Vec<Context>);

impl IntegerContexts {
    fn new() -> Self {
        Self(vec![Context::default(); 512])
    }

    /// Encodes `value`, or OOB for `None`.
    fn encode(&mut self, encoder: &mut MqEncoder, value: Option<i32>) {
        let (negative, magnitude) = match value {
            Some(value) => (value < 0, value.unsigned_abs()),
            None => (true, 0),
        };
        // (prefix ones, value bits, offset) for each range of A.2.
        const RANGES: [(u32, u32, u32); 6] = [(0, 2, 0), (1, 4, 4), (2, 6, 20), (3, 8, 84), (4, 12, 340), (5, 32, 4436)];
        let &(ones, bits, offset) = RANGES
            .iter()
            .find(|&&(_, bits, offset)| bits == 32 || magnitude < offset + (1 << bits))
            .unwrap();

        let mut prev = 1usize;
        let mut put = |encoder: &mut MqEncoder, bit: bool| {
            encoder.encode(&mut self.0[prev], bit);
            let next = (prev << 1) | bit as usize;
            prev = if prev < 256 { next } else { (next & 511) | 256 };
        };
        put(encoder, negative);
        for _ in 0..ones {
            put(encoder, true);
        }
        if ones < 5 {
            put(encoder, false);
        }
        let value = magnitude - offset;
        for shift in (0..bits).rev() {
            put(encoder, (value >> shift) & 1 == 1);
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Context {
    index: u8,
    mps: bool,
}

/// MQ arithmetic encoder (T.88 E.2), terminated like jbig2enc with 0xFF 0xAC.
struct MqEncoder {
    a: u32,
    c: u32,
    ct: u32,
    /// `out[0]` is the byte before the first output byte (BPST - 1).
    out: Vec<u8>,
}

/// Qe, NMPS, NLPS and SWITCH (T.88 Table E.1).
const QE: [(u32, u8, u8, bool); 47] = [
    (0x5601, 1, 1, true), (0x3401, 2, 6, false), (0x1801, 3, 9, false), (0x0AC1, 4, 12, false),
    (0x0521, 5, 29, false), (0x0221, 38, 33, false), (0x5601, 7, 6, true), (0x5401, 8, 14, false),
    (0x4801, 9, 14, false), (0x3801, 10, 14, false), (0x3001, 11, 17, false), (0x2401, 12, 18, false),
    (0x1C01, 13, 20, false), (0x1601, 29, 21, false), (0x5601, 15, 14, true), (0x5401, 16, 14, false),
    (0x5101, 17, 15, false), (0x4801, 18, 16, false), (0x3801, 19, 17, false), (0x3401, 20, 18, false),
    (0x3001, 21, 19, false), (0x2801, 22, 19, false), (0x2401, 23, 20, false), (0x2201, 24, 21, false),
    (0x1C01, 25, 22, false), (0x1801, 26, 23, false), (0x1601, 27, 24, false), (0x1401, 28, 25, false),
    (0x1201, 29, 26, false), (0x1101, 30, 27, false), (0x0AC1, 31, 28, false), (0x09C1, 32, 29, false),
    (0x08A1, 33, 30, false), (0x0521, 34, 31, false), (0x0441, 35, 32, false), (0x02A1, 36, 33, false),
    (0x0221, 37, 34, false), (0x0141, 38, 35, false), (0x0111, 39, 36, false), (0x0085, 40, 37, false),
    (0x0049, 41, 38, false), (0x0025, 42, 39, false), (0x0015, 43, 40, false), (0x0009, 44, 41, false),
    (0x0005, 45, 42, false), (0x0001, 45, 43, false), (0x5601, 46, 46, false),
];

impl MqEncoder {
    fn new() -> Self {
        Self { a: 0x8000, c: 0, ct: 12, out: vec![0] }
    }

    fn encode(&mut self, context: &mut Context, bit: bool) {
        let (qe, nmps, nlps, switch) = QE[context.index as usize];
        self.a -= qe;
        if bit == context.mps {
            if self.a & 0x8000 != 0 {
                self.c += qe;
                return;
            }
            if self.a < qe {
                self.a = qe;
            } else {
                self.c += qe;
            }
            context.index = nmps;
        } else {
            if self.a < qe {
                self.c += qe;
            } else {
                self.a = qe;
            }
            if switch {
                context.mps = !context.mps;
            }
            context.index = nlps;
        }
        loop {
            self.a <<= 1;
            self.c <<= 1;
            self.ct -= 1;
            if self.ct == 0 {
                self.byte_out();
            }
            if self.a & 0x8000 != 0 {
                break;
            }
        }
    }

    fn byte_out(&mut self) {
        let last = self.out.len() - 1;
        if self.out[last] != 0xFF && self.c >= 0x800_0000 {
            self.out[last] += 1;
            self.c &= 0x7FF_FFFF;
        }
        if self.out[last] == 0xFF {
            self.out.push((self.c >> 20) as u8);
            self.c &= 0xF_FFFF;
            self.ct = 7;
        } else {
            self.out.push((self.c >> 19) as u8);
            self.c &= 0x7_FFFF;
            self.ct = 8;
        }
    }

    fn finish(mut self) -> Vec<u8> {
        let temp = self.c + self.a;
        self.c |= 0xFFFF;
        if self.c >= temp {
            self.c -= 0x8000;
        }
        self.c <<= self.ct;
        self.byte_out();
        self.c <<= self.ct;
        self.byte_out();
        if self.out.last() != Some(&0xFF) {
            self.out.push(0xFF);
        }
        self.out.push(0xAC);
        assert_eq!(self.out[0], 0, "a carry reached the byte before the stream");
        self.out.remove(0);
        self.out
    }
}
