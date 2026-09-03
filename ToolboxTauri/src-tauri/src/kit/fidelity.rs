use image::codecs::gif::GifDecoder;
use image::AnimationDecoder;
use lopdf::Document;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContentShape {
    SingleFrame { width: u32, height: u32 },
    FrameSequence { count: usize, width: u32, height: u32 },
    PageSequence { count: usize },
    Unsupported { reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OrientationPolicy {
    BakePixelsOnce,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetadataPolicy {
    PreserveUnlessToolRemoves,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FidelityModel {
    pub input_path: PathBuf,
    pub shape: ContentShape,
    pub orientation: OrientationPolicy,
    pub metadata: MetadataPolicy,
}

pub fn describe(path: &Path) -> Result<FidelityModel, String> {
    let shape = match path.extension().and_then(|extension| extension.to_str()).map(|extension| extension.to_ascii_lowercase()).as_deref() {
        Some("pdf") => {
            let document = Document::load(path).map_err(|error| format!("Could not read PDF: {error}"))?;
            ContentShape::PageSequence { count: document.get_pages().len() }
        }
        Some("gif") => describe_gif(path)?,
        Some("tif" | "tiff") => ContentShape::Unsupported {
            reason: "TIFF page sequences require a page-aware TIFF operation.".to_string(),
        },
        _ => {
            let (width, height) = image::image_dimensions(path).map_err(|error| format!("Could not read image dimensions: {error}"))?;
            ContentShape::SingleFrame { width, height }
        }
    };

    Ok(FidelityModel {
        input_path: path.to_path_buf(),
        shape,
        orientation: OrientationPolicy::BakePixelsOnce,
        metadata: MetadataPolicy::PreserveUnlessToolRemoves,
    })
}

fn describe_gif(path: &Path) -> Result<ContentShape, String> {
    let file = File::open(path).map_err(|error| format!("Could not read GIF: {error}"))?;
    let frames = GifDecoder::new(BufReader::new(file))
        .map_err(|error| format!("Could not decode GIF: {error}"))?
        .into_frames()
        .collect_frames()
        .map_err(|error| format!("Could not read GIF frames: {error}"))?;
    let Some(first) = frames.first() else {
        return Err("GIF has no frames.".to_string());
    };
    let (width, height) = first.buffer().dimensions();
    if frames.len() == 1 {
        Ok(ContentShape::SingleFrame { width, height })
    } else {
        Ok(ContentShape::FrameSequence { count: frames.len(), width, height })
    }
}
