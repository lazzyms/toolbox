use crate::kit::contracts::{JobOutcome, LimitKind, ToolError};
use flate2::read::ZlibDecoder;
use std::cell::RefCell;
use std::io::{self, Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

const MB: u64 = 1_000_000;
const GB: u64 = 1_000_000_000;
const FIVE_MINUTES: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolLimits {
    pub max_bytes: u64,
    pub max_pages: Option<u32>,
    pub max_expanded_bytes: u64,
    pub max_duration: Duration,
}

// Expansion stays under the image crate's 512 MiB allocation default so this check, not a decoder error, reports it.
const PDF: ToolLimits = ToolLimits { max_bytes: GB, max_pages: Some(5_000), max_expanded_bytes: 500 * MB, max_duration: FIVE_MINUTES };
const DOCUMENT: ToolLimits = ToolLimits { max_pages: None, ..PDF };
const IMAGE: ToolLimits = ToolLimits { max_bytes: 250 * MB, max_pages: None, max_expanded_bytes: 500 * MB, max_duration: FIVE_MINUTES };
const PAGED_IMAGE: ToolLimits = ToolLimits { max_pages: Some(1_000), ..IMAGE };

pub fn recorded(command: &str) -> Option<ToolLimits> {
    Some(match command {
        "remove_password" | "protect_pdf" | "export_pdf_scene" | "crop_pdf" | "sign_pdf" | "edit_pdf" | "edit_pdf_session"
        | "organize_pdf" | "add_pdf_pages" | "add_page_numbers" | "watermark_pdf" | "compress_pdf" | "remove_pdf_pages"
        | "extract_pdf_pages" | "merge_pdfs" | "split_pdf" | "pdf_to_images" | "pdf_to_text" | "extract_pdf_images"
        | "ocr_pdf" => PDF,
        "protect_office" => DOCUMENT,
        "compress_images" | "convert_images" | "export_image_edit_plan" | "images_to_pdf" | "blur_faces"
        | "remove_image_background" | "resize_images" | "rotate_images" | "crop_images" | "adjust_image_tone"
        | "watermark_images" | "generate_icon_set" | "create_gif" => IMAGE,
        "extract_gif_frames" | "process_tiff_pages" | "image_metadata" => PAGED_IMAGE,
        _ => return None,
    })
}

impl ToolLimits {
    pub fn run_each<F>(self, input: PathBuf, job: F) -> JobOutcome
    where
        F: FnOnce(PathBuf) -> JobOutcome + Send + 'static,
    {
        run_timed(input.clone(), self.max_duration, move || match self.check(&input) {
            Ok(()) => job(input),
            Err(error) => JobOutcome::failure(input, error),
        })
    }

    pub fn run_all<F>(self, inputs: &[PathBuf], job: F) -> JobOutcome
    where
        F: FnOnce() -> JobOutcome + Send + 'static,
    {
        let inputs = inputs.to_vec();
        let first = inputs.first().cloned().unwrap_or_default();
        run_timed(first, self.max_duration, move || {
            match inputs.iter().find_map(|input| self.check(input).err().map(|error| (input, error))) {
                Some((input, error)) => JobOutcome::failure(input.clone(), error),
                None => job(),
            }
        })
    }

    pub fn check(&self, path: &Path) -> Result<(), ToolError> {
        let Ok(metadata) = std::fs::metadata(path) else { return Ok(()) };
        let format = Format::for_path(path);
        if metadata.len() > self.max_bytes {
            return Err(ToolError::limit_exceeded(
                LimitKind::FileSize,
                format!("Larger than the {} limit for one {}. Reduce its size and try again.", bytes_label(self.max_bytes), format.noun()),
            ));
        }
        let Ok(bytes) = std::fs::read(path) else { return Ok(()) };
        let shape = Shape::read(&bytes, self.max_pages.unwrap_or(u32::MAX), self.max_expanded_bytes);
        if shape.largest_part > self.max_expanded_bytes {
            return Err(ToolError::limit_exceeded(
                LimitKind::Decompression,
                format!(
                    "{} expands to more than {} when opened, which is over the limit. It may be damaged or unsafe to open.",
                    format.part(), bytes_label(self.max_expanded_bytes),
                ),
            ));
        }
        match self.max_pages {
            Some(max_pages) if shape.pages > u64::from(max_pages) => Err(ToolError::limit_exceeded(
                LimitKind::PageCount,
                format!(
                    "More than {} {}, over the limit for one {}. Use fewer {} and try again.",
                    count_label(u64::from(max_pages)), format.pages(), format.noun(), format.pages(),
                ),
            )),
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Copy)]
enum Format { Pdf, Document, Gif, Tiff, Image }

impl Format {
    fn for_path(path: &Path) -> Self {
        let extension = path.extension().and_then(|extension| extension.to_str()).unwrap_or_default().to_ascii_lowercase();
        match extension.as_str() {
            "pdf" => Self::Pdf,
            "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" => Self::Document,
            "gif" => Self::Gif,
            "tif" | "tiff" => Self::Tiff,
            _ => Self::Image,
        }
    }

    fn noun(self) -> &'static str {
        match self { Self::Pdf => "PDF", Self::Document => "document", Self::Gif => "GIF", Self::Tiff => "TIFF", Self::Image => "image" }
    }

    fn pages(self) -> &'static str {
        if matches!(self, Self::Gif) { "frames" } else { "pages" }
    }

    fn part(self) -> &'static str {
        match self {
            Self::Pdf => "Part of this PDF",
            Self::Document => "Part of this document",
            Self::Gif => "Each frame of this GIF",
            Self::Tiff => "A page of this TIFF",
            Self::Image => "This image",
        }
    }
}

struct Shape {
    pages: u64,
    largest_part: u64,
}

impl Shape {
    // The bytes decide the reader, not the extension, so a renamed file is still measured.
    fn read(bytes: &[u8], max_pages: u32, max_expanded: u64) -> Self {
        let head = &bytes[..bytes.len().min(1024)];
        if head.windows(5).any(|window| window == b"%PDF-") {
            let largest_part = pdf_largest_stream(bytes, max_expanded);
            let pages = if largest_part > max_expanded { 0 } else {
                lopdf::Document::load_metadata_mem(bytes).map(|metadata| u64::from(metadata.page_count)).unwrap_or(0)
            };
            Self { pages, largest_part }
        } else if bytes.starts_with(b"PK\x03\x04") {
            Self { pages: 0, largest_part: zip_largest_entry(bytes, max_expanded) }
        } else if bytes.starts_with(b"GIF8") {
            gif_shape(bytes, max_pages)
        } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
            tiff_shape(bytes, max_pages)
        } else if bytes.get(4..8) == Some(b"ftyp") {
            let largest_part = heif::probe(bytes)
                .map(|info| u64::from(info.width) * u64::from(info.height) * u64::from(info.color_type.bytes_per_pixel()))
                .unwrap_or(0);
            Self { pages: 1, largest_part }
        } else {
            Self { pages: 1, largest_part: raster_bytes(bytes) }
        }
    }
}

fn inflated_len(reader: impl Read, max_expanded: u64) -> u64 {
    let mut limited = reader.take(max_expanded + 1);
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0;
    while let Ok(read @ 1..) = limited.read(&mut buffer) {
        total += read as u64;
    }
    total
}

fn pdf_largest_stream(bytes: &[u8], max_expanded: u64) -> u64 {
    let mut largest = 0;
    let mut cursor = 0;
    while let Some(found) = bytes[cursor..].windows(6).position(|window| window == b"stream") {
        let keyword = cursor + found;
        cursor = keyword + 6;
        if keyword >= 3 && &bytes[keyword - 3..keyword] == b"end" { continue; }
        let mut start = cursor;
        if bytes.get(start) == Some(&b'\r') { start += 1; }
        if bytes.get(start) == Some(&b'\n') { start += 1; }
        let Some(&[method, flags]) = bytes.get(start..start + 2) else { continue };
        if method & 0x0F != 8 || (u16::from(method) << 8 | u16::from(flags)) % 31 != 0 { continue; }
        largest = largest.max(inflated_len(ZlibDecoder::new(&bytes[start..]), max_expanded));
        if largest > max_expanded { break; }
    }
    largest
}

fn zip_largest_entry(bytes: &[u8], max_expanded: u64) -> u64 {
    let Ok(mut archive) = zip::ZipArchive::new(Cursor::new(bytes)) else { return 0 };
    let mut largest = 0;
    for index in 0..archive.len() {
        let Ok(entry) = archive.by_index(index) else { continue };
        largest = largest.max(inflated_len(entry, max_expanded));
        if largest > max_expanded { break; }
    }
    largest
}

fn gif_shape(bytes: &[u8], max_pages: u32) -> Shape {
    let Some(screen) = bytes.get(6..13) else { return Shape { pages: 0, largest_part: 0 } };
    let width = u64::from(u16::from_le_bytes([screen[0], screen[1]]));
    let height = u64::from(u16::from_le_bytes([screen[2], screen[3]]));
    let largest_part = width * height * 4;
    let mut position = 13 + color_table_len(screen[4]);
    let mut pages = 0;
    while pages <= u64::from(max_pages) {
        match bytes.get(position) {
            Some(0x21) => position = skip_sub_blocks(bytes, position + 2),
            Some(0x2C) => {
                let Some(&packed) = bytes.get(position + 9) else { break };
                pages += 1;
                position = skip_sub_blocks(bytes, position + 10 + color_table_len(packed) + 1);
            }
            _ => break,
        }
    }
    Shape { pages, largest_part }
}

fn color_table_len(packed: u8) -> usize {
    if packed & 0x80 == 0 { 0 } else { 3 << ((packed & 0x07) + 1) }
}

fn skip_sub_blocks(bytes: &[u8], mut position: usize) -> usize {
    while let Some(&length) = bytes.get(position) {
        position += 1 + usize::from(length);
        if length == 0 { break; }
    }
    position
}

fn tiff_shape(bytes: &[u8], max_pages: u32) -> Shape {
    let mut shape = Shape { pages: 0, largest_part: 0 };
    let Ok(mut decoder) = tiff::decoder::Decoder::new(Cursor::new(bytes)) else { return shape };
    loop {
        shape.pages += 1;
        if let (Ok((width, height)), Ok(color)) = (decoder.dimensions(), decoder.colortype()) {
            let bits = u64::from(color.bit_depth()) * u64::from(color.num_samples());
            shape.largest_part = shape.largest_part.max((u64::from(width) * u64::from(height) * bits).div_ceil(8));
        }
        if shape.pages > u64::from(max_pages) || !decoder.more_images() || decoder.next_image().is_err() { break; }
    }
    shape
}

fn raster_bytes(bytes: &[u8]) -> u64 {
    use image::ImageDecoder;
    let Ok(mut reader) = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format() else { return 0 };
    reader.no_limits();
    reader.into_decoder().map(|decoder| decoder.total_bytes()).unwrap_or(0)
}

struct Clock {
    published: Mutex<Option<Vec<PathBuf>>>,
}

thread_local! {
    static CLOCK: RefCell<Option<Arc<Clock>>> = const { RefCell::new(None) };
}

pub fn publish_within_time(publish: impl FnOnce() -> io::Result<PathBuf>) -> io::Result<PathBuf> {
    let Some(clock) = CLOCK.with(|clock| clock.borrow().clone()) else { return publish() };
    let mut published = clock.published.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(paths) = published.as_mut() else {
        return Err(io::Error::new(io::ErrorKind::TimedOut, "The time limit was reached before this output was saved."));
    };
    let path = publish()?;
    paths.push(path.clone());
    Ok(path)
}

// A thread cannot be killed, so a job past its limit keeps running detached and every later publish is refused.
fn run_timed<F>(input: PathBuf, max_duration: Duration, job: F) -> JobOutcome
where
    F: FnOnce() -> JobOutcome + Send + 'static,
{
    let clock = Arc::new(Clock { published: Mutex::new(Some(Vec::new())) });
    let worker_clock = Arc::clone(&clock);
    let (sender, receiver) = mpsc::channel();
    let spawned = std::thread::Builder::new().name("toolbox-job".into()).spawn(move || {
        CLOCK.with(|clock| *clock.borrow_mut() = Some(worker_clock));
        let _ = sender.send(job());
    });
    if let Err(error) = spawned {
        return JobOutcome::failure(input, ToolError::processing(format!("Could not start processing: {error}")));
    }
    let result = receiver.recv_timeout(max_duration);
    let saved = || clock.published.lock().unwrap_or_else(PoisonError::into_inner).take().unwrap_or_default();
    match result {
        Ok(outcome) => outcome,
        Err(RecvTimeoutError::Timeout) => {
            let saved = saved();
            let message = format!("Took longer than {} and was stopped. {}", duration_label(max_duration), saved_label(saved.len()));
            JobOutcome::failure_with_outputs(input, saved, ToolError::limit_exceeded(LimitKind::Time, message))
        }
        Err(RecvTimeoutError::Disconnected) => {
            let saved = saved();
            let message = format!("Processing stopped unexpectedly. {}", saved_label(saved.len()));
            JobOutcome::failure_with_outputs(input, saved, ToolError::processing(message))
        }
    }
}

fn saved_label(count: usize) -> String {
    match count {
        0 => "Nothing was saved.".to_string(),
        1 => "1 part was saved before it stopped.".to_string(),
        count => format!("{count} parts were saved before it stopped."),
    }
}

fn bytes_label(bytes: u64) -> String {
    match bytes {
        bytes if bytes >= GB && bytes % GB == 0 => format!("{} GB", count_label(bytes / GB)),
        bytes if bytes >= MB && bytes % MB == 0 => format!("{} MB", count_label(bytes / MB)),
        bytes if bytes >= 1_000 && bytes % 1_000 == 0 => format!("{} KB", count_label(bytes / 1_000)),
        bytes => format!("{} bytes", count_label(bytes)),
    }
}

fn duration_label(duration: Duration) -> String {
    let plural = |count: u128, unit: &str| if count == 1 { format!("1 {unit}") } else { format!("{count} {unit}s") };
    match duration.as_millis() {
        millis if millis >= 60_000 && millis % 60_000 == 0 => plural(millis / 60_000, "minute"),
        millis if millis >= 1_000 && millis % 1_000 == 0 => plural(millis / 1_000, "second"),
        millis => plural(millis, "millisecond"),
    }
}

fn count_label(count: u64) -> String {
    let digits = count.to_string();
    let mut label = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 { label.push(','); }
        label.push(digit);
    }
    label
}

#[cfg(test)]
pub mod fixtures {
    use std::io::Write;

    // Fixed-Huffman deflate of a zero byte followed by length-258, distance-1 copies: about 13 bits per 258 bytes.
    pub fn zlib_of_zeros(len: u64) -> Vec<u8> {
        let mut bits = BitWriter::default();
        bits.push(0b1, 1);
        bits.push(0b01, 2);
        bits.push_code(0b0011_0000, 8);
        let copies = (len.saturating_sub(1)).div_ceil(258);
        for _ in 0..copies {
            bits.push_code(0b1100_0101, 8);
            bits.push_code(0, 5);
        }
        bits.push_code(0, 7);
        let mut stream = vec![0x78, 0x01];
        stream.extend(bits.finish());
        let zeros = 1 + 258 * copies;
        stream.extend_from_slice(&((((zeros % 65_521) as u32) << 16) | 1).to_be_bytes());
        stream
    }

    #[derive(Default)]
    struct BitWriter { bytes: Vec<u8>, current: u8, filled: u8 }

    impl BitWriter {
        fn push(&mut self, value: u32, count: u8) {
            for bit in 0..count {
                self.current |= (((value >> bit) & 1) as u8) << self.filled;
                self.filled += 1;
                if self.filled == 8 { self.bytes.push(self.current); self.current = 0; self.filled = 0; }
            }
        }

        fn push_code(&mut self, code: u32, count: u8) {
            for bit in (0..count).rev() { self.push((code >> bit) & 1, 1); }
        }

        fn finish(mut self) -> Vec<u8> {
            if self.filled > 0 { self.bytes.push(self.current); }
            self.bytes
        }
    }

    pub fn pdf_with_stream(stream: &[u8]) -> Vec<u8> {
        let mut pdf = Vec::new();
        write!(pdf, "%PDF-1.7\n1 0 obj\n<< /Length {} /Filter /FlateDecode >>\nstream\n", stream.len()).unwrap();
        pdf.extend_from_slice(stream);
        pdf.extend_from_slice(b"\nendstream\nendobj\n%%EOF\n");
        pdf
    }

    pub fn png_header(width: u32, height: u32) -> Vec<u8> {
        let chunk = |kind: &[u8], data: &[u8]| {
            let mut crc = flate2::Crc::new();
            crc.update(kind);
            crc.update(data);
            let mut chunk = (data.len() as u32).to_be_bytes().to_vec();
            chunk.extend_from_slice(kind);
            chunk.extend_from_slice(data);
            chunk.extend_from_slice(&crc.sum().to_be_bytes());
            chunk
        };
        let mut header = width.to_be_bytes().to_vec();
        header.extend_from_slice(&height.to_be_bytes());
        header.extend_from_slice(&[8, 6, 0, 0, 0]);
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend(chunk(b"IHDR", &header));
        png.extend(chunk(b"IDAT", &zlib_of_zeros(1)));
        png.extend(chunk(b"IEND", &[]));
        png
    }

    pub fn gif_with_frames(width: u16, height: u16, frames: usize) -> Vec<u8> {
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&width.to_le_bytes());
        gif.extend_from_slice(&height.to_le_bytes());
        gif.extend_from_slice(&[0x80, 0, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF]);
        for _ in 0..frames {
            gif.extend_from_slice(&[0x21, 0xF9, 4, 0, 10, 0, 0, 0]);
            gif.push(0x2C);
            gif.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0, 0]);
            gif.extend_from_slice(&[2, 2, 0x44, 0x01, 0]);
        }
        gif.push(0x3B);
        gif
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use crate::kit::contracts::{native_capabilities, ErrorKind, OutcomeState};
    use std::io::Write;
    use std::time::Instant;

    fn sandbox(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("toolbox-limits-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn small() -> ToolLimits {
        ToolLimits { max_bytes: 10 * MB, max_pages: Some(3), max_expanded_bytes: 1_000, max_duration: Duration::from_secs(5) }
    }

    fn limit_of(result: Result<(), ToolError>) -> Option<LimitKind> {
        match result {
            Err(ToolError { kind: ErrorKind::LimitExceeded { limit }, .. }) => Some(limit),
            Err(error) => panic!("expected a limit, got {error:?}"),
            Ok(()) => None,
        }
    }

    #[test]
    fn every_registered_command_records_limits_and_pages_only_where_files_have_pages() {
        for capability in native_capabilities() {
            let limits = recorded(&capability.command).unwrap_or_else(|| panic!("{} has no recorded limits", capability.command));
            let paged = capability.accepted_extensions.iter().any(|extension| matches!(extension.as_str(), ".pdf" | ".gif" | ".tif" | ".tiff"));
            assert_eq!(limits.max_pages.is_some(), paged, "{}", capability.command);
            assert_eq!(limits.max_duration, FIVE_MINUTES, "{}", capability.command);
        }
        assert_eq!(recorded("compress_pdf"), Some(ToolLimits { max_bytes: GB, max_pages: Some(5_000), max_expanded_bytes: 500 * MB, max_duration: FIVE_MINUTES }));
        assert_eq!(recorded("resize_images"), Some(ToolLimits { max_bytes: 250 * MB, max_pages: None, max_expanded_bytes: 500 * MB, max_duration: FIVE_MINUTES }));
        assert_eq!(recorded("extract_gif_frames").and_then(|limits| limits.max_pages), Some(1_000));
        assert_eq!(recorded("unknown"), None);
    }

    #[test]
    fn messages_state_the_recorded_values_in_plain_units() {
        assert_eq!(bytes_label(GB), "1 GB");
        assert_eq!(bytes_label(250 * MB), "250 MB");
        assert_eq!(bytes_label(1_234), "1,234 bytes");
        assert_eq!(duration_label(FIVE_MINUTES), "5 minutes");
        assert_eq!(duration_label(Duration::from_millis(40)), "40 milliseconds");
        assert_eq!(count_label(5_000), "5,000");
        assert_eq!(count_label(1_000_000), "1,000,000");
    }

    #[test]
    fn zlib_fixture_inflates_to_the_requested_length() {
        let mut inflated = Vec::new();
        ZlibDecoder::new(&zlib_of_zeros(10_000)[..]).read_to_end(&mut inflated).unwrap();
        assert!(inflated.len() >= 10_000 && inflated.len() < 10_000 + 258);
        assert!(inflated.iter().all(|byte| *byte == 0));
    }

    #[test]
    fn each_format_is_measured_against_its_expansion_and_page_limits() {
        let root = sandbox("formats");
        let write = |name: &str, bytes: &[u8]| { let path = root.join(name); std::fs::write(&path, bytes).unwrap(); path };
        let limits = small();

        assert_eq!(limit_of(limits.check(&write("small.pdf", &pdf_with_stream(&zlib_of_zeros(700))))), None);
        assert_eq!(limit_of(limits.check(&write("bomb.pdf", &pdf_with_stream(&zlib_of_zeros(1_100))))), Some(LimitKind::Decompression));

        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file("word/document.xml", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(&[b'a'; 1_100]).unwrap();
        let zip = zip.finish().unwrap().into_inner();
        assert_eq!(limit_of(limits.check(&write("bomb.docx", &zip))), Some(LimitKind::Decompression));

        assert_eq!(limit_of(limits.check(&write("wide.png", &png_header(16, 15)))), None);
        assert_eq!(limit_of(limits.check(&write("wide-renamed.webp", &png_header(16, 16)))), Some(LimitKind::Decompression));

        assert_eq!(limit_of(limits.check(&write("three.gif", &gif_with_frames(4, 4, 3)))), None);
        assert_eq!(limit_of(limits.check(&write("four.gif", &gif_with_frames(4, 4, 4)))), Some(LimitKind::PageCount));
        assert_eq!(limit_of(limits.check(&write("large.gif", &gif_with_frames(16, 16, 1)))), Some(LimitKind::Decompression));

        let mut cursor = Cursor::new(Vec::new());
        {
            let mut encoder = tiff::encoder::TiffEncoder::new(&mut cursor).unwrap();
            for _ in 0..4 { encoder.write_image::<tiff::encoder::colortype::Gray8>(2, 2, &[0; 4]).unwrap(); }
        }
        assert_eq!(limit_of(limits.check(&write("four.tiff", cursor.get_ref()))), Some(LimitKind::PageCount));

        let message = limits.check(&write("four-frames.gif", &gif_with_frames(4, 4, 4))).unwrap_err().message;
        assert_eq!(message, "More than 3 frames, over the limit for one GIF. Use fewer frames and try again.");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_job_past_its_time_limit_reports_saved_parts_and_cannot_publish_more() {
        let root = sandbox("timed");
        let input = root.join("slow.pdf");
        std::fs::write(&input, b"%PDF-1.7\n").unwrap();
        let (first, late) = (root.join("slow-1.txt"), root.join("slow-2.txt"));
        let (late_sender, late_result) = mpsc::channel();
        let limits = ToolLimits { max_duration: Duration::from_millis(100), ..small() };
        let started = Instant::now();
        let outcome = limits.run_each(input.clone(), {
            let (first, late) = (first.clone(), late.clone());
            move |input| {
                publish_within_time(|| std::fs::write(&first, b"1").map(|()| first.clone())).unwrap();
                std::thread::sleep(Duration::from_millis(400));
                let _ = late_sender.send(publish_within_time(|| std::fs::write(&late, b"2").map(|()| late.clone())));
                JobOutcome::succeeded(input, vec![first, late], "done")
            }
        });

        assert!(started.elapsed() < Duration::from_millis(350), "the runner answers at the limit, not when the job ends");
        match &outcome.state {
            OutcomeState::Failed { output_paths, error } => {
                assert_eq!(output_paths, &vec![first.clone()]);
                assert_eq!(error.kind, ErrorKind::LimitExceeded { limit: LimitKind::Time });
                assert_eq!(error.message, "Took longer than 100 milliseconds and was stopped. 1 part was saved before it stopped.");
            }
            state => panic!("expected a time limit failure, got {state:?}"),
        }
        assert!(late_result.recv().unwrap().is_err(), "a publish after the limit is refused");
        assert!(!late.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_panicking_job_fails_only_its_own_file() {
        let outcome = small().run_each(PathBuf::from("missing.png"), |_| panic!("decoder bug"));
        assert!(matches!(outcome.error(), Some(ToolError { kind: ErrorKind::Processing, .. })));
        assert_eq!(outcome.error().unwrap().message, "Processing stopped unexpectedly. Nothing was saved.");
    }
}
