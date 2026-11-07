//! Throwaway spike test for #623: decodes the code-generated JBIG2 fixtures
//! with hayro-jbig2, which is a dev-dependency only. Replace with production
//! coverage in #630. See `jbig2-decoder-decision.md`.

use hayro_jbig2::{DecodeSettings, Globals, Page};
use lopdf::{Document, Object};

use super::jbig2_fixtures::{self, Jbig2Fixture};

/// Collects decoded pixels row by row; `true` is black.
#[derive(Default)]
struct Pixels {
    rows: Vec<Vec<bool>>,
    row: Vec<bool>,
}

impl hayro_jbig2::Decoder for Pixels {
    fn push_pixel(&mut self, black: bool) {
        self.row.push(black);
    }

    fn push_pixel_chunk(&mut self, black: bool, chunk_count: u32) {
        self.row.extend(std::iter::repeat(black).take(chunk_count as usize * 8));
    }

    fn next_line(&mut self) {
        self.rows.push(std::mem::take(&mut self.row));
    }
}

fn decode(stream: &[u8], globals: Option<&[u8]>) -> Result<(u32, u32, Vec<bool>), hayro_jbig2::DecodeError> {
    let settings = DecodeSettings { strict: true };
    let globals = globals.map(|data| Globals::new(data, &settings)).transpose()?;
    let page = Page::new_embedded(stream, globals.as_ref(), &settings)?;
    let mut pixels = Pixels::default();
    page.decode(&mut pixels)?;
    let (width, height) = (page.width(), page.height());
    assert!(pixels.row.is_empty(), "pixels after the last row");
    assert_eq!(pixels.rows.len(), height as usize);
    // Chunks may pad a row to a byte boundary; keep the image width.
    let black = pixels
        .rows
        .into_iter()
        .flat_map(|mut row| {
            assert!(row.len() >= width as usize);
            row.truncate(width as usize);
            row
        })
        .collect();
    Ok((width, height, black))
}

fn assert_decodes(fixture: &Jbig2Fixture, stream: &[u8], globals: Option<&[u8]>) {
    let (width, height, black) =
        decode(stream, globals).unwrap_or_else(|error| panic!("{}: {error:?}", fixture.name));
    assert_eq!((width, height), (fixture.width, fixture.height), "{}", fixture.name);
    assert!(black.iter().any(|&pixel| pixel), "{}: blank fixture", fixture.name);
    assert_eq!(render(&black, width), render(&fixture.black, width), "{}", fixture.name);
}

fn render(pixels: &[bool], width: u32) -> String {
    pixels
        .chunks(width as usize)
        .map(|row| row.iter().map(|&black| if black { '#' } else { '.' }).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn hayro_jbig2_decodes_each_fixture_exactly() {
    for fixture in jbig2_fixtures::all() {
        assert_decodes(&fixture, &fixture.stream, fixture.globals.as_deref());
    }
}

#[test]
fn embedded_globals_fixture_needs_its_globals() {
    let fixture = jbig2_fixtures::text_region_with_embedded_globals();
    assert!(fixture.globals.is_some());
    assert!(decode(&fixture.stream, None).is_err(), "decoded a text region without its symbol dictionary");
}

#[test]
fn hayro_jbig2_decodes_jbig2_images_read_back_from_a_pdf() {
    let fixtures = jbig2_fixtures::all();
    let mut bytes = Vec::new();
    jbig2_fixtures::pdf_with_images(&fixtures).save_to(&mut bytes).unwrap();
    let document = Document::load_mem(&bytes).unwrap();

    let pages = document.get_pages();
    assert_eq!(pages.len(), fixtures.len());
    for ((_, page_id), fixture) in pages.into_iter().zip(&fixtures) {
        let (resources, _) = document.get_page_resources(page_id).unwrap();
        let xobjects = resources.unwrap().get(b"XObject").unwrap().as_dict().unwrap();
        let image_id = xobjects.get(b"Im0").unwrap().as_reference().unwrap();
        let image = document.get_object(image_id).unwrap().as_stream().unwrap();
        assert_eq!(image.dict.get(b"Filter").unwrap().as_name().unwrap(), b"JBIG2Decode");

        let globals = match image.dict.get(b"DecodeParms") {
            Ok(parms) => {
                let globals_id = parms.as_dict().unwrap().get(b"JBIG2Globals").unwrap().as_reference().unwrap();
                Some(document.get_object(globals_id).and_then(Object::as_stream).unwrap().content.clone())
            }
            Err(_) => None,
        };
        assert_eq!(globals.is_some(), fixture.globals.is_some(), "{}", fixture.name);
        assert_decodes(fixture, &image.content, globals.as_deref());
    }
}
