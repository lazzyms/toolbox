//! Removing image metadata. JPEG and PNG are cleaned by copying every image segment or chunk as it
//! is and leaving the metadata ones out, so the pixels are never re-encoded. The orientation and
//! colour profile stay, so a cleaned photo looks and turns the same as the original.

use super::metadata::ImageMetadata;

/// The reason a file is Skipped when the stack would only remove metadata and the file has none.
pub const NO_METADATA: &str = "No metadata to remove. No copy was made.";
pub const CLEANED_LOSSLESSLY: &str = "Cleaned without re-encoding.";
pub const REENCODED_TO_CLEAN: &str = "Re-encoded at quality 90. HEIC, WebP and TIFF can't be cleaned without it.";
pub const LOCATION_REMAINED: &str = "Location details were still present after cleaning, so no copy was kept.";
pub const METADATA_REMAINED: &str = "Metadata was still present after cleaning, so no copy was kept.";

/// The kinds of metadata a file holds that cleaning removes, named as the results list them.
pub(crate) fn removable(found: &ImageMetadata) -> Vec<&'static str> {
    [(found.camera_details, "camera details"), (found.gps, "GPS location"), (found.xmp, "XMP"), (found.iptc, "IPTC"), (found.comments, "comments")]
        .into_iter()
        .filter_map(|(present, name)| present.then_some(name))
        .collect()
}

/// Checks a saved copy. A copy that still holds anything cleaning removes fails, and location is
/// named on its own because it is the detail people most need gone.
pub(crate) fn verify(saved: &ImageMetadata) -> Result<(), &'static str> {
    if saved.gps {
        Err(LOCATION_REMAINED)
    } else if removable(saved).is_empty() {
        Ok(())
    } else {
        Err(METADATA_REMAINED)
    }
}

/// The note for a cleaned file: what was removed and what was kept.
pub(crate) fn summary(source: &ImageMetadata, saved: &ImageMetadata) -> String {
    let removed = removable(source);
    let kept = [(source.orientation.is_some(), "orientation"), (source.icc && saved.icc, "colour profile")]
        .into_iter()
        .filter_map(|(present, name)| present.then_some(name))
        .collect::<Vec<_>>();
    let mut parts = Vec::new();
    if !removed.is_empty() { parts.push(format!("Removed: {}.", sentence(&removed))); }
    if !kept.is_empty() { parts.push(format!("Kept: {}.", sentence(&kept))); }
    parts.join(" ")
}

fn sentence(names: &[&str]) -> String {
    let joined = names.join(", ");
    let mut chars = joined.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// A big-endian EXIF block holding nothing but the orientation tag.
fn orientation_exif(orientation: u8) -> Vec<u8> {
    let mut tiff = b"MM\0\x2A\0\0\0\x08".to_vec();
    tiff.extend(1u16.to_be_bytes());
    tiff.extend(0x0112u16.to_be_bytes());
    tiff.extend(3u16.to_be_bytes());
    tiff.extend(1u32.to_be_bytes());
    tiff.extend(u16::from(orientation).to_be_bytes());
    tiff.extend([0, 0]);
    tiff.extend(0u32.to_be_bytes());
    tiff
}

fn be16(data: &[u8], at: usize) -> Option<usize> { data.get(at..at + 2).map(|b| u16::from_be_bytes([b[0], b[1]]) as usize) }
fn be32(data: &[u8], at: usize) -> Option<usize> { data.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize) }

/// Copies a JPEG without its EXIF, XMP, IPTC, comments and other application segments. JFIF, the
/// ICC colour profile and Adobe's colour transform stay, and the scan data is copied byte for byte
/// up to the end-of-image marker. An orientation-only EXIF block replaces the original's.
pub(crate) fn clean_jpeg(data: &[u8], orientation: Option<u8>) -> Result<Vec<u8>, String> {
    let broken = || "Could not clean this JPEG: its segments are damaged.".to_string();
    if !data.starts_with(&[0xFF, 0xD8]) { return Err(broken()); }
    let mut out = vec![0xFF, 0xD8];
    let mut exif = orientation.map(|value| [b"Exif\0\0".as_slice(), &orientation_exif(value)].concat());
    let mut pos = 2;
    loop {
        if data.get(pos) != Some(&0xFF) { return Err(broken()); }
        while data.get(pos) == Some(&0xFF) { pos += 1; }
        let marker = *data.get(pos).ok_or_else(broken)?;
        pos += 1;
        if marker == 0xDA {
            let scan = pos - 2;
            let end = data[scan..].windows(2).position(|pair| pair == [0xFF, 0xD9]).ok_or_else(broken)? + scan + 2;
            if let Some(exif) = exif.take() { push_segment(&mut out, 0xE1, &exif)?; }
            out.extend_from_slice(&data[scan..end]);
            return Ok(out);
        }
        if matches!(marker, 0x01 | 0xD0..=0xD7) { continue; }
        let length = be16(data, pos).filter(|length| *length >= 2).ok_or_else(broken)?;
        let body = data.get(pos + 2..pos + length).ok_or_else(broken)?;
        pos += length;
        let keep = match marker {
            0xE0 => body.starts_with(b"JFIF\0"),
            0xE2 => body.starts_with(b"ICC_PROFILE\0"),
            0xEE => body.starts_with(b"Adobe"),
            0xE1 | 0xE3..=0xED | 0xEF | 0xFE => false,
            _ => true,
        };
        // EXIF belongs straight after SOI, or after JFIF when there is one.
        if !(marker == 0xE0 && keep) {
            if let Some(exif) = exif.take() { push_segment(&mut out, 0xE1, &exif)?; }
        }
        if keep { push_segment(&mut out, marker, body)?; }
    }
}

fn push_segment(out: &mut Vec<u8>, marker: u8, body: &[u8]) -> Result<(), String> {
    let length = u16::try_from(body.len() + 2).map_err(|_| "Could not clean this JPEG: a segment is too long.".to_string())?;
    out.extend([0xFF, marker]);
    out.extend(length.to_be_bytes());
    out.extend_from_slice(body);
    Ok(())
}

/// Copies a PNG without its eXIf chunk and its text chunks, which carry XMP, IPTC, raw EXIF and
/// comments. A text chunk that only carries the ICC colour profile stays, as do iCCP and every
/// image chunk. An orientation-only eXIf chunk goes straight after IHDR.
pub(crate) fn clean_png(data: &[u8], orientation: Option<u8>) -> Result<Vec<u8>, String> {
    let broken = || "Could not clean this PNG: its chunks are damaged.".to_string();
    let mut out = data.get(..8).ok_or_else(broken)?.to_vec();
    let mut pos = 8;
    loop {
        let length = be32(data, pos).ok_or_else(broken)?;
        let whole = data.get(pos..pos + 12 + length).ok_or_else(broken)?;
        let (kind, chunk) = (&whole[4..8], &whole[8..8 + length]);
        pos += 12 + length;
        let keep = match kind {
            b"eXIf" => false,
            b"tEXt" | b"zTXt" | b"iTXt" => matches!(chunk.split(|byte| *byte == 0).next().unwrap_or_default(), b"Raw profile type icc" | b"Raw profile type icm"),
            _ => true,
        };
        if keep { out.extend_from_slice(whole); }
        match kind {
            b"IHDR" => if let Some(value) = orientation { push_chunk(&mut out, b"eXIf", &orientation_exif(value)); },
            b"IEND" => return Ok(out),
            _ => {}
        }
    }
}

fn push_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    let mut crc = flate2::Crc::new();
    crc.update(kind);
    crc.update(data);
    out.extend((data.len() as u32).to_be_bytes());
    out.extend(kind);
    out.extend_from_slice(data);
    out.extend(crc.sum().to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::images::metadata::{self, fixtures::{self, Wanted}};

    const EVERYTHING: Wanted = Wanted { exif: true, gps: true, orientation: Some(6), xmp: true, iptc: true, comments: true, icc: true };

    fn decoded(bytes: &[u8]) -> image::RgbImage { image::load_from_memory(bytes).unwrap().to_rgb8() }

    #[test]
    fn jpeg_cleaning_keeps_the_scan_orientation_and_profile() {
        let source = fixtures::jpeg(EVERYTHING);
        let cleaned = clean_jpeg(&source, Some(6)).unwrap();
        let found = metadata::read(&cleaned).unwrap();
        assert!(!found.camera_details && !found.gps && !found.xmp && !found.iptc && !found.comments, "{found:?}");
        assert_eq!((found.orientation, found.icc), (Some(6), true));
        assert_eq!(decoded(&cleaned), decoded(&source), "the scan must be copied, not re-encoded");
        assert!(verify(&found).is_ok());
    }

    #[test]
    fn png_cleaning_keeps_the_pixels_orientation_and_profile() {
        let source = fixtures::png(EVERYTHING);
        let cleaned = clean_png(&source, Some(6)).unwrap();
        let found = metadata::read(&cleaned).unwrap();
        assert!(!found.camera_details && !found.gps && !found.xmp && !found.iptc && !found.comments, "{found:?}");
        assert_eq!((found.orientation, found.icc), (Some(6), true));
        assert_eq!(decoded(&cleaned), decoded(&source));
    }

    #[test]
    fn cleaning_without_an_orientation_writes_no_exif() {
        let cleaned = clean_jpeg(&fixtures::jpeg(Wanted { exif: true, ..Wanted::default() }), None).unwrap();
        assert!(!metadata::read(&cleaned).unwrap().exif);
        let cleaned = clean_png(&fixtures::png(Wanted { comments: true, ..Wanted::default() }), None).unwrap();
        assert!(!metadata::read(&cleaned).unwrap().exif);
    }

    #[test]
    fn an_orientation_only_block_is_not_metadata_to_remove() {
        let found = metadata::read(&clean_jpeg(&fixtures::jpeg(Wanted::default()), Some(3)).unwrap()).unwrap();
        assert!(found.exif && removable(&found).is_empty());
        assert_eq!(removable(&metadata::read(&fixtures::jpeg(EVERYTHING)).unwrap()), ["camera details", "GPS location", "XMP", "IPTC", "comments"]);
    }

    #[test]
    fn a_copy_that_still_holds_metadata_fails_verification() {
        assert_eq!(verify(&metadata::read(&fixtures::jpeg(Wanted { gps: true, ..Wanted::default() })).unwrap()), Err(LOCATION_REMAINED));
        assert_eq!(verify(&metadata::read(&fixtures::png(Wanted { xmp: true, ..Wanted::default() })).unwrap()), Err(METADATA_REMAINED));
    }

    #[test]
    fn the_summary_names_what_was_removed_and_kept() {
        let source = metadata::read(&fixtures::jpeg(Wanted { exif: true, gps: true, orientation: Some(6), xmp: true, icc: true, ..Wanted::default() })).unwrap();
        let saved = metadata::read(&clean_jpeg(&fixtures::jpeg(EVERYTHING), Some(6)).unwrap()).unwrap();
        assert_eq!(summary(&source, &saved), "Removed: Camera details, GPS location, XMP. Kept: Orientation, colour profile.");
    }
}
