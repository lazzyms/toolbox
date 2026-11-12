//! Extract GIF Frames: every frame of a GIF as a full-canvas PNG plus one
//! timings manifest that lists each frame file and its delay.

use image::codecs::gif::GifDecoder;
use image::codecs::png::PngEncoder;
use image::{AnimationDecoder, ExtendedColorType, ImageEncoder};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use super::tools::GifExtractRequest;
use crate::kit::common::{JobOutcome, OutputNaming, OutputReservation};
use crate::kit::contracts::ToolError;

pub const FRAME_SUFFIX: &str = "-frame";
pub const TIMINGS_SUFFIX: &str = "-timings";
const NOTHING_SAVED: &str = "No frames were saved for this GIF.";

struct DecodedFrame {
    png: Vec<u8>,
    delay_ms: u32,
}

/// Decodes the whole GIF before anything is written, so a damaged GIF leaves
/// no partial frames behind. Frames come back from the decoder composited
/// onto the full canvas with each frame's disposal applied.
pub fn extract(request: &GifExtractRequest, input: PathBuf) -> JobOutcome {
    let (width, height, frames) = match decode(&input) {
        Ok(decoded) => decoded,
        Err(error) => return JobOutcome::failure(input, error),
    };
    let width_digits = frames.len().to_string().len().max(3);
    let frame_names: Vec<String> = (1..=frames.len()).map(|number| format!("{FRAME_SUFFIX}-{number:0width_digits$}")).collect();
    let (frame_slots, timings_slot) = match reserve_set(&input, &request.output_location, &frame_names) {
        Ok(slots) => slots,
        Err(error) => return JobOutcome::failure(input, ToolError::processing(format!("Could not reserve the frame files: {error}"))),
    };

    let mut manifest_frames = Vec::with_capacity(frames.len());
    for (frame, slot) in frames.iter().zip(&frame_slots) {
        if let Err(error) = std::fs::write(slot.path(), &frame.png) {
            return JobOutcome::failure(input, ToolError::processing(format!("Could not save a frame: {error}")));
        }
        manifest_frames.push(serde_json::json!({ "file": file_name(slot.destination_path()), "delayMs": frame.delay_ms }));
    }
    let manifest = serde_json::json!({
        "gif": file_name(&input),
        "width": width,
        "height": height,
        "frames": manifest_frames,
    });
    let manifest_bytes = match serde_json::to_vec_pretty(&manifest) {
        Ok(bytes) => bytes,
        Err(error) => return JobOutcome::failure(input, ToolError::processing(format!("Could not write the timings file: {error}"))),
    };
    if let Err(error) = std::fs::write(timings_slot.path(), manifest_bytes) {
        return JobOutcome::failure(input, ToolError::processing(format!("Could not save the timings file: {error}")));
    }

    let mut outputs = Vec::with_capacity(frame_slots.len() + 1);
    for slot in frame_slots.into_iter().chain(std::iter::once(timings_slot)) {
        match slot.publish() {
            Ok(path) => outputs.push(path),
            Err(error) => {
                return JobOutcome::failure_with_outputs(input, outputs, ToolError::processing(format!("Could not save the GIF's frames: {error}")));
            }
        }
    }
    let count = frames.len();
    let pngs = if count == 1 { "1 PNG".to_string() } else { format!("{count} PNGs") };
    JobOutcome::succeeded(input, outputs, format!("{pngs}, {width} × {height}"))
}

fn decode(input: &Path) -> Result<(u32, u32, Vec<DecodedFrame>), ToolError> {
    let file = File::open(input).map_err(|error| ToolError::processing(format!("Could not open this GIF: {error}")))?;
    let decoder = GifDecoder::new(BufReader::new(file))
        .map_err(|_| ToolError::invalid_input(format!("This file can't be read as a GIF. {NOTHING_SAVED}")))?;
    let (width, height) = image::ImageDecoder::dimensions(&decoder);
    let mut frames = Vec::new();
    for (index, frame) in decoder.into_frames().enumerate() {
        let frame = frame.map_err(|_| {
            ToolError::invalid_input(format!("Frame {} is damaged, so the GIF can't be read past it. {NOTHING_SAVED}", index + 1))
        })?;
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        let delay_ms = (u64::from(numerator) / u64::from(denominator.max(1))) as u32;
        let buffer = frame.into_buffer();
        let mut png = Vec::new();
        PngEncoder::new(&mut png)
            .write_image(buffer.as_raw(), buffer.width(), buffer.height(), ExtendedColorType::Rgba8)
            .map_err(|error| ToolError::processing(format!("Could not encode frame {}: {error}", index + 1)))?;
        frames.push(DecodedFrame { png, delay_ms });
    }
    if frames.is_empty() {
        return Err(ToolError::invalid_input(format!("This GIF has no frames. {NOTHING_SAVED}")));
    }
    Ok((width, height, frames))
}

/// Claims every frame name and the timings name with one shared collision
/// counter, so a GIF's files always carry matching numbers and nothing that
/// already exists is replaced.
fn reserve_set(
    input: &Path,
    location: &crate::kit::common::OutputLocation,
    frame_suffixes: &[String],
) -> std::io::Result<(Vec<OutputReservation>, OutputReservation)> {
    let directory = location.directory_for(input);
    let stem = input.file_stem().and_then(|value| value.to_str()).unwrap_or("");
    let collision = |counter: usize| if counter == 0 { String::new() } else { format!("-{counter}") };
    'counter: for counter in 0usize.. {
        let tail = collision(counter);
        let mut frames = Vec::with_capacity(frame_suffixes.len());
        for suffix in frame_suffixes {
            match OutputNaming::reserve_named_candidate(&directory.join(format!("{stem}{suffix}{tail}.png")))? {
                Some(slot) => frames.push(slot),
                None => continue 'counter,
            }
        }
        if let Some(timings) = OutputNaming::reserve_named_candidate(&directory.join(format!("{stem}{TIMINGS_SUFFIX}{tail}.json")))? {
            return Ok((frames, timings));
        }
    }
    unreachable!("the collision counter is unbounded")
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::common::batch_runner::{BatchRunner, RunEvents};
    use crate::kit::common::OutputLocation;
    use crate::kit::contracts::ErrorKind;
    use gif::{DisposalMethod, Encoder, Frame, Repeat};
    use std::borrow::Cow;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const CLEAR: [u8; 4] = [0, 0, 0, 0];
    // Palette index 0 red, 1 green, 2 blue, 3 transparent.
    const PALETTE: [u8; 12] = [255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0];

    struct Folder(PathBuf);
    impl Folder {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("toolbox_gif_extract_{}_{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn join(&self, name: &str) -> PathBuf { self.0.join(name) }
        fn names(&self) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(&self.0).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
            names.sort();
            names
        }
    }
    impl Drop for Folder { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }

    struct Patch { left: u16, top: u16, width: u16, height: u16, index: u8, delay_cs: u16, dispose: DisposalMethod }

    /// A 4×4 GIF with a red full-canvas first frame and the given patches after it.
    fn write_gif(path: &Path, first_delay_cs: u16, first_dispose: DisposalMethod, patches: &[Patch]) {
        let mut bytes = Vec::new();
        {
            let mut encoder = Encoder::new(&mut bytes, 4, 4, &PALETTE).unwrap();
            encoder.set_repeat(Repeat::Infinite).unwrap();
            let base = Frame { width: 4, height: 4, delay: first_delay_cs, dispose: first_dispose, transparent: Some(3), buffer: Cow::Owned(vec![0; 16]), ..Frame::default() };
            encoder.write_frame(&base).unwrap();
            for patch in patches {
                let frame = Frame {
                    left: patch.left, top: patch.top, width: patch.width, height: patch.height,
                    delay: patch.delay_cs, dispose: patch.dispose, transparent: Some(3),
                    buffer: Cow::Owned(vec![patch.index; usize::from(patch.width) * usize::from(patch.height)]),
                    ..Frame::default()
                };
                encoder.write_frame(&frame).unwrap();
            }
        }
        std::fs::write(path, bytes).unwrap();
    }

    fn pixel(path: &Path, x: u32, y: u32) -> [u8; 4] {
        image::open(path).unwrap().to_rgba8().get_pixel(x, y).0
    }

    fn request() -> GifExtractRequest { GifExtractRequest { paths: Vec::new(), output_location: OutputLocation::AlongsideInput } }

    fn manifest(path: &Path) -> serde_json::Value { serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap() }

    #[test]
    fn every_frame_is_a_padded_full_canvas_png_that_honours_disposal() {
        let folder = Folder::new("disposal");
        let input = folder.join("spinner.gif");
        write_gif(&input, 12, DisposalMethod::Keep, &[
            // Frame 2: green top-left 2×2, cleared to transparent afterwards.
            Patch { left: 0, top: 0, width: 2, height: 2, index: 1, delay_cs: 5, dispose: DisposalMethod::Background },
            // Frame 3: blue bottom-right 2×2, then restored to what was under it.
            Patch { left: 2, top: 2, width: 2, height: 2, index: 2, delay_cs: 30, dispose: DisposalMethod::Previous },
            // Frame 4: green 1×1 at (3,0) kept.
            Patch { left: 3, top: 0, width: 1, height: 1, index: 1, delay_cs: 100, dispose: DisposalMethod::Keep },
        ]);
        let original = std::fs::read(&input).unwrap();

        let outcome = extract(&request(), input.clone());
        assert!(outcome.error().is_none(), "{outcome:?}");
        assert_eq!(outcome.detail, "4 PNGs, 4 × 4");
        let names: Vec<String> = outcome.output_paths().iter().map(|path| file_name(path)).collect();
        assert_eq!(names, ["spinner-frame-001.png", "spinner-frame-002.png", "spinner-frame-003.png", "spinner-frame-004.png", "spinner-timings.json"]);

        let frames = &outcome.output_paths()[..4];
        for frame in frames {
            assert_eq!(image::image_dimensions(frame).unwrap(), (4, 4), "every frame is the full canvas");
        }
        // Frame 1 is all red.
        assert_eq!(pixel(&frames[0], 0, 0), RED);
        assert_eq!(pixel(&frames[0], 3, 3), RED);
        // Frame 2 draws green over the kept red frame.
        assert_eq!(pixel(&frames[1], 0, 0), GREEN);
        assert_eq!(pixel(&frames[1], 3, 3), RED);
        // Frame 3: frame 2's area was cleared by Background disposal, blue is drawn bottom-right.
        assert_eq!(pixel(&frames[2], 0, 0), CLEAR);
        assert_eq!(pixel(&frames[2], 1, 1), CLEAR);
        assert_eq!(pixel(&frames[2], 3, 3), BLUE);
        assert_eq!(pixel(&frames[2], 3, 0), RED);
        // Frame 4: frame 3's blue was restored to red by Previous disposal; the cleared area stays clear.
        assert_eq!(pixel(&frames[3], 3, 3), RED);
        assert_eq!(pixel(&frames[3], 0, 0), CLEAR);
        assert_eq!(pixel(&frames[3], 3, 0), GREEN);

        assert_eq!(manifest(&outcome.output_paths()[4]), serde_json::json!({
            "gif": "spinner.gif",
            "width": 4,
            "height": 4,
            "frames": [
                { "file": "spinner-frame-001.png", "delayMs": 120 },
                { "file": "spinner-frame-002.png", "delayMs": 50 },
                { "file": "spinner-frame-003.png", "delayMs": 300 },
                { "file": "spinner-frame-004.png", "delayMs": 1000 },
            ],
        }));
        assert_eq!(std::fs::read(&input).unwrap(), original, "the original GIF is unchanged");
        assert!(folder.names().iter().all(|name| !name.starts_with('.')), "no temporary files are left: {:?}", folder.names());
    }

    #[test]
    fn padding_grows_past_three_digits_for_long_gifs() {
        let folder = Folder::new("padding");
        let input = folder.join("long.gif");
        let patches: Vec<Patch> = (0..1000).map(|index| Patch { left: 0, top: 0, width: 1, height: 1, index: (index % 3) as u8, delay_cs: 2, dispose: DisposalMethod::Keep }).collect();
        write_gif(&input, 2, DisposalMethod::Keep, &patches);

        let outcome = extract(&request(), input);
        assert!(outcome.error().is_none(), "{outcome:?}");
        let paths = outcome.output_paths();
        assert_eq!(paths.len(), 1002);
        assert_eq!(file_name(&paths[0]), "long-frame-0001.png");
        assert_eq!(file_name(&paths[1000]), "long-frame-1001.png");
        let listed = manifest(&paths[1001]);
        assert_eq!(listed["frames"].as_array().unwrap().len(), 1001);
        assert_eq!(listed["frames"][998]["file"], "long-frame-0999.png");
    }

    #[test]
    fn a_still_gif_gives_one_frame_and_a_timings_file() {
        let folder = Folder::new("still");
        let input = folder.join("logo.gif");
        write_gif(&input, 0, DisposalMethod::Keep, &[]);

        let outcome = extract(&request(), input);
        assert!(outcome.error().is_none(), "{outcome:?}");
        assert_eq!(outcome.detail, "1 PNG, 4 × 4");
        let names: Vec<String> = outcome.output_paths().iter().map(|path| file_name(path)).collect();
        assert_eq!(names, ["logo-frame-001.png", "logo-timings.json"]);
        assert_eq!(pixel(&outcome.output_paths()[0], 2, 2), RED);
        assert_eq!(manifest(&outcome.output_paths()[1])["frames"], serde_json::json!([{ "file": "logo-frame-001.png", "delayMs": 0 }]));
    }

    #[test]
    fn existing_names_are_never_replaced_and_a_gifs_files_share_one_number() {
        let folder = Folder::new("collision");
        let input = folder.join("wave.gif");
        write_gif(&input, 10, DisposalMethod::Keep, &[Patch { left: 0, top: 0, width: 1, height: 1, index: 1, delay_cs: 10, dispose: DisposalMethod::Keep }]);
        std::fs::write(folder.join("wave-frame-002.png"), b"keep me").unwrap();

        let outcome = extract(&request(), input);
        assert!(outcome.error().is_none(), "{outcome:?}");
        let names: Vec<String> = outcome.output_paths().iter().map(|path| file_name(path)).collect();
        assert_eq!(names, ["wave-frame-001-1.png", "wave-frame-002-1.png", "wave-timings-1.json"]);
        assert_eq!(std::fs::read(folder.join("wave-frame-002.png")).unwrap(), b"keep me");
        assert!(!folder.join("wave-frame-001.png").exists(), "a half-claimed set is released");
        assert_eq!(manifest(&outcome.output_paths()[2])["frames"][1]["file"], "wave-frame-002-1.png");
    }

    #[test]
    fn a_gif_damaged_part_way_saves_nothing_and_names_the_frame() {
        let folder = Folder::new("damaged");
        let healthy = folder.join("healthy.gif");
        write_gif(&healthy, 10, DisposalMethod::Keep, &[
            Patch { left: 0, top: 0, width: 4, height: 4, index: 1, delay_cs: 10, dispose: DisposalMethod::Keep },
            Patch { left: 0, top: 0, width: 4, height: 4, index: 2, delay_cs: 10, dispose: DisposalMethod::Keep },
        ]);
        let mut bytes = std::fs::read(&healthy).unwrap();
        // Cut the file inside the last frame's image data.
        bytes.truncate(bytes.len() - 3);
        let damaged = folder.join("damaged.gif");
        std::fs::write(&damaged, &bytes).unwrap();
        std::fs::remove_file(&healthy).unwrap();

        let outcome = extract(&request(), damaged.clone());
        let error = outcome.error().expect("a damaged GIF fails");
        assert_eq!(error.kind, ErrorKind::InvalidInput);
        assert_eq!(error.message, "Frame 3 is damaged, so the GIF can't be read past it. No frames were saved for this GIF.");
        assert!(outcome.output_paths().is_empty());
        assert_eq!(folder.names(), ["damaged.gif"], "no frames or temporary files are left");
        assert_eq!(std::fs::read(&damaged).unwrap(), bytes);
    }

    #[test]
    fn a_batch_with_one_corrupt_gif_still_extracts_the_others() {
        let folder = Folder::new("batch");
        let wave = folder.join("wave.gif");
        let spin = folder.join("spin.gif");
        let broken = folder.join("broken.gif");
        write_gif(&wave, 8, DisposalMethod::Keep, &[Patch { left: 1, top: 1, width: 2, height: 2, index: 1, delay_cs: 16, dispose: DisposalMethod::Background }]);
        write_gif(&spin, 4, DisposalMethod::Keep, &[
            Patch { left: 0, top: 0, width: 2, height: 2, index: 2, delay_cs: 4, dispose: DisposalMethod::Previous },
            Patch { left: 2, top: 2, width: 2, height: 2, index: 1, delay_cs: 20, dispose: DisposalMethod::Keep },
        ]);
        // A small, well-formed header followed by bytes that are not a GIF block.
        std::fs::write(&broken, b"GIF89a\x04\x00\x04\x00\x00\x00\x00\x99 not a frame").unwrap();
        let originals: Vec<Vec<u8>> = [&wave, &spin, &broken].iter().map(|path| std::fs::read(path).unwrap()).collect();

        let request = std::sync::Arc::new(GifExtractRequest { paths: vec![wave.clone(), broken.clone(), spin.clone()], output_location: OutputLocation::AlongsideInput });
        let outcomes = BatchRunner::run(&RunEvents::silent(), "extract_gif_frames", request.paths.clone(), &request.output_location, &request, super::super::tools::gif_extract);

        assert_eq!(outcomes.len(), 3);
        let by_input = |path: &Path| outcomes.iter().find(|outcome| outcome.input_path == path).unwrap();
        assert_eq!(by_input(&wave).output_paths().iter().map(|path| file_name(path)).collect::<Vec<_>>(), ["wave-frame-001.png", "wave-frame-002.png", "wave-timings.json"]);
        assert_eq!(by_input(&spin).output_paths().iter().map(|path| file_name(path)).collect::<Vec<_>>(), ["spin-frame-001.png", "spin-frame-002.png", "spin-frame-003.png", "spin-timings.json"]);
        assert_eq!(manifest(&by_input(&spin).output_paths()[3])["frames"][2]["delayMs"], 200);
        let failed = by_input(&broken);
        assert_eq!(failed.error().unwrap().kind, ErrorKind::InvalidInput);
        assert!(failed.error().unwrap().message.ends_with("No frames were saved for this GIF."), "{failed:?}");
        assert!(failed.output_paths().is_empty());
        assert!(!folder.names().iter().any(|name| name.starts_with("broken-")));
        for (path, bytes) in [&wave, &spin, &broken].iter().zip(&originals) {
            assert_eq!(&std::fs::read(path).unwrap(), bytes, "{} is unchanged", path.display());
        }
    }
}
