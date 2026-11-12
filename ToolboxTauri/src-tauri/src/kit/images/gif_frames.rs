//! Create GIF frame geometry and palette.
//!
//! The canvas is the first frame's size. Every frame is centred on a
//! transparent canvas: smaller frames are padded, larger frames are scaled
//! down to fit keeping their aspect ratio. Each frame is then quantised to a
//! local palette whose index 0 is reserved for transparency.
//!
//! Everything here is integer arithmetic or plain IEEE `f32` arithmetic with no
//! transcendental functions and no randomness, so the same frames give the
//! same bytes on every supported target.

use image::imageops::FilterType;
use image::RgbaImage;
use std::collections::{BTreeMap, HashMap};

/// The palette index every frame reserves for transparent pixels.
pub(crate) const TRANSPARENT_INDEX: u8 = 0;

/// A pixel with alpha below this is transparent; at or above it is opaque.
const OPAQUE_FROM: u8 = 128;

/// Opaque colours a frame can use, after the reserved transparent index.
const OPAQUE_COLOURS: usize = 255;

/// The largest size that fits inside `canvas` with the aspect ratio of `size`.
/// Sizes that already fit are returned unchanged; nothing is scaled up.
pub(crate) fn fit_within((width, height): (u32, u32), (canvas_width, canvas_height): (u32, u32)) -> (u32, u32) {
    if width <= canvas_width && height <= canvas_height { return (width, height); }
    let (w, h, cw, ch) = (width as u64, height as u64, canvas_width as u64, canvas_height as u64);
    if w * ch >= h * cw {
        (canvas_width, ((h * cw + w / 2) / w).clamp(1, ch) as u32)
    } else {
        (((w * ch + h / 2) / h).clamp(1, cw) as u32, canvas_height)
    }
}

/// Places `frame` centred on a transparent canvas, scaling it down first when it is larger.
pub(crate) fn place_on_canvas(frame: &RgbaImage, canvas: (u32, u32)) -> RgbaImage {
    let (width, height) = fit_within(frame.dimensions(), canvas);
    let scaled;
    let source = if (width, height) == frame.dimensions() {
        frame
    } else {
        // Triangle is plain arithmetic, unlike Lanczos which calls sin().
        scaled = image::imageops::resize(frame, width, height, FilterType::Triangle);
        &scaled
    };
    let mut placed = RgbaImage::new(canvas.0, canvas.1);
    let left = (canvas.0 - width) / 2;
    let top = (canvas.1 - height) / 2;
    image::imageops::replace(&mut placed, source, left as i64, top as i64);
    placed
}

pub(crate) struct QuantisedFrame {
    /// RGB triples; entry 0 is the reserved transparent colour.
    pub palette: Vec<u8>,
    pub indices: Vec<u8>,
}

const fn key(r: u8, g: u8, b: u8) -> u32 { (r as u32) << 16 | (g as u32) << 8 | b as u32 }
const fn channel(colour: u32, which: usize) -> u8 { (colour >> (16 - 8 * which)) as u8 }

/// Quantises a frame to at most 255 opaque colours plus the reserved transparent index.
/// A frame with 255 or fewer opaque colours keeps them exactly; otherwise a
/// count-weighted median cut picks the palette.
pub(crate) fn quantise(frame: &RgbaImage) -> QuantisedFrame {
    let mut histogram = BTreeMap::<u32, u64>::new();
    for pixel in frame.pixels() {
        let [r, g, b, a] = pixel.0;
        if a >= OPAQUE_FROM { *histogram.entry(key(r, g, b)).or_default() += 1; }
    }
    let colours: Vec<u32> = if histogram.len() <= OPAQUE_COLOURS {
        histogram.keys().copied().collect()
    } else {
        median_cut(histogram.into_iter().collect())
    };

    let mut palette = Vec::with_capacity((colours.len() + 1) * 3);
    palette.extend_from_slice(&[0, 0, 0]);
    for colour in &colours { palette.extend_from_slice(&[channel(*colour, 0), channel(*colour, 1), channel(*colour, 2)]); }

    let mut nearest = HashMap::<u32, u8>::new();
    let indices = frame.pixels().map(|pixel| {
        let [r, g, b, a] = pixel.0;
        if a < OPAQUE_FROM { return TRANSPARENT_INDEX; }
        let wanted = key(r, g, b);
        *nearest.entry(wanted).or_insert_with(|| 1 + nearest_colour(&colours, wanted))
    }).collect();
    QuantisedFrame { palette, indices }
}

/// The index of the closest colour; the lowest index wins a tie.
fn nearest_colour(colours: &[u32], wanted: u32) -> u8 {
    let distance = |colour: u32| (0..3).map(|which| {
        let difference = channel(colour, which) as i32 - channel(wanted, which) as i32;
        (difference * difference) as u32
    }).sum::<u32>();
    let mut best = (u32::MAX, 0usize);
    for (index, colour) in colours.iter().enumerate() {
        let candidate = distance(*colour);
        if candidate < best.0 { best = (candidate, index); }
    }
    best.1 as u8
}

struct ColourBox { entries: Vec<(u32, u64)>, widest: usize, range: u8, population: u64 }

impl ColourBox {
    fn new(entries: Vec<(u32, u64)>) -> Self {
        let mut widest = 0;
        let mut range = 0;
        for which in 0..3 {
            let (low, high) = entries.iter().fold((u8::MAX, u8::MIN), |(low, high), (colour, _)| {
                let value = channel(*colour, which);
                (low.min(value), high.max(value))
            });
            let spread = high.saturating_sub(low);
            if spread > range { (widest, range) = (which, spread); }
        }
        let population = entries.iter().map(|(_, count)| count).sum();
        ColourBox { entries, widest, range, population }
    }

    fn average(&self) -> u32 {
        let mean = |which: usize| {
            let sum = self.entries.iter().map(|(colour, count)| channel(*colour, which) as u64 * count).sum::<u64>();
            ((sum + self.population / 2) / self.population) as u8
        };
        key(mean(0), mean(1), mean(2))
    }
}

fn median_cut(histogram: Vec<(u32, u64)>) -> Vec<u32> {
    let mut boxes = vec![ColourBox::new(histogram)];
    while boxes.len() < OPAQUE_COLOURS {
        // Split the box with the widest channel; ties go to the more populous, then the earlier box.
        let Some(target) = boxes.iter().enumerate()
            .filter(|(_, colour_box)| colour_box.entries.len() > 1)
            .max_by(|(left_index, left), (right_index, right)| {
                (left.range, left.population).cmp(&(right.range, right.population)).then(right_index.cmp(left_index))
            })
            .map(|(index, _)| index)
        else { break };
        let ColourBox { mut entries, widest, population, .. } = boxes.remove(target);
        entries.sort_unstable_by_key(|(colour, _)| (channel(*colour, widest), *colour));
        let mut running = 0;
        let mut split = entries.len() - 1;
        for (index, (_, count)) in entries.iter().enumerate() {
            running += count;
            if running * 2 >= population { split = index + 1; break; }
        }
        let upper = entries.split_off(split.clamp(1, entries.len() - 1));
        boxes.insert(target, ColourBox::new(upper));
        boxes.insert(target, ColourBox::new(entries));
    }
    boxes.iter().map(ColourBox::average).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn fit_keeps_smaller_frames_and_scales_larger_ones_down_with_their_aspect() {
        assert_eq!(fit_within((20, 10), (40, 30)), (20, 10));
        assert_eq!(fit_within((80, 30), (40, 30)), (40, 15));
        assert_eq!(fit_within((30, 90), (40, 30)), (10, 30));
        assert_eq!(fit_within((120, 90), (40, 30)), (40, 30));
        assert_eq!(fit_within((4000, 1), (40, 30)), (40, 1));
    }

    #[test]
    fn small_frames_keep_exact_colours_and_transparency_takes_index_zero() {
        let frame = RgbaImage::from_fn(4, 2, |x, _| match x { 0 => Rgba([9, 9, 9, 0]), 1 => Rgba([200, 10, 10, 255]), 2 => Rgba([10, 200, 10, 127]), _ => Rgba([10, 10, 200, 128]) });
        let quantised = quantise(&frame);
        assert_eq!(quantised.palette, vec![0, 0, 0, 10, 10, 200, 200, 10, 10]);
        assert_eq!(quantised.indices, vec![0, 2, 0, 1, 0, 2, 0, 1]);
    }

    #[test]
    fn many_colours_fill_255_opaque_entries_and_never_use_the_transparent_index() {
        let frame = RgbaImage::from_fn(64, 64, |x, y| Rgba([(x * 4) as u8, (y * 4) as u8, ((x + y) * 2) as u8, if x == 0 { 0 } else { 255 }]));
        let quantised = quantise(&frame);
        assert_eq!(quantised.palette.len(), 256 * 3);
        for (pixel, index) in frame.pixels().zip(&quantised.indices) {
            assert_eq!(pixel.0[3] == 0, *index == TRANSPARENT_INDEX);
        }
        assert_eq!(quantise(&frame).indices, quantised.indices);
    }
}
