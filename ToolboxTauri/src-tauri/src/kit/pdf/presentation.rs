//! The slide model shared by the PowerPoint (PPTX) and OpenDocument presentation (ODP) readers.

use crate::kit::pdf::font_resolver::EmbeddedFont;
use crate::kit::pdf::layout::{Block, Omitted, Picture};

/// A 10 by 7.5 inch slide, used when a deck states no slide size.
pub(super) const DEFAULT_SLIDE: (f32, f32) = (720., 540.);
pub(super) const MIN_SLIDE_SIDE: f32 = 72.;
pub(super) const MAX_SLIDE_SIDE: f32 = 14_400.;

/// Which optional parts of a deck to read. Hidden slides and notes left out here are never read into the output.
#[derive(Debug, Clone, Copy, Default)]
pub struct Include { pub hidden: bool, pub notes: bool }

/// One slide to print: its shapes as page-anchored floats, and its speaker notes when they were asked for.
#[derive(Debug, Default)]
pub struct Slide {
    /// The slide's position in the deck, counted from 1, hidden slides included.
    pub number: usize,
    pub size: (f32, f32),
    pub title: Option<String>,
    pub shapes: Vec<Block>,
    pub notes: Vec<Block>,
    /// Shapes drawn as their bounding box because they are rotated or not rectangular.
    pub approximate: usize,
    pub omitted: Omitted,
}

pub struct Deck {
    pub slides: Vec<Slide>,
    /// Hidden slides that were not read because hidden slides were not asked for.
    pub hidden_left_out: usize,
    pub pictures: Vec<Picture>,
    pub families: Vec<String>,
    pub body_family: Option<String>,
    pub embedded: Vec<EmbeddedFont>,
    pub title: Option<String>,
    pub author: Option<String>,
}

/// A slide side in points, kept to a printable range.
pub(super) fn slide_side(points: f32) -> f32 { points.clamp(MIN_SLIDE_SIDE, MAX_SLIDE_SIDE) }
