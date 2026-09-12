use lopdf::{Document, Object};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfPageMetadata {
    pub index: usize,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub rotation: i32,
    pub page_box: [f32; 4],
    pub preview: Option<String>,
    pub text_runs: Option<Vec<PdfTextRun>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfTextRun {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfDocumentMetadata {
    pub path: PathBuf,
    pub pages: Vec<PdfPageMetadata>,
}

pub fn inspect(path: &Path) -> Result<PdfDocumentMetadata, String> {
    let document = Document::load(path).map_err(|error| format!("Could not read PDF: {error}"))?;
    let pages = document
        .get_pages()
        .values()
        .enumerate()
        .map(|(index, page_id)| page_bounds(&document, *page_id).and_then(|(left, bottom, right, top)| {
            let crop = super::inherited(&document, *page_id, b"CropBox")?
                .map(|value| value.as_array().map_err(|error| error.to_string()).and_then(|values| {
                    if values.len() != 4 { return Err("PDF crop box must have four values.".to_string()); }
                    Ok([
                        number(super::resolve(&document, &values[0])?)?, number(super::resolve(&document, &values[1])?)?,
                        number(super::resolve(&document, &values[2])?)?, number(super::resolve(&document, &values[3])?)?,
                    ])
                }))
                .transpose()?
                .unwrap_or([left, bottom, right, top]);
            let page_box = [crop[0].max(left), crop[1].max(bottom), crop[2].min(right), crop[3].min(top)];
            if page_box[2] <= page_box[0] || page_box[3] <= page_box[1] { return Err("PDF crop box has invalid dimensions.".to_string()); }
            let rotation = super::inherited(&document, *page_id, b"Rotate")?
                .map(|value| value.as_i64().map_err(|error| error.to_string()))
                .transpose()?
                .unwrap_or(0)
                .rem_euclid(360) as i32;
            Ok(PdfPageMetadata {
                index,
                x: left,
                y: bottom,
                width: right - left,
                height: top - bottom,
                rotation,
                page_box,
                preview: render_preview(path, index + 1),
                text_runs: None,
            })
        }))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PdfDocumentMetadata { path: path.to_path_buf(), pages })
}

fn render_preview(path: &Path, page: usize) -> Option<String> {
    let renderer = find_pdftoppm()?;
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_nanos();
    let prefix = std::env::temp_dir().join(format!("toolbox-pdf-preview-{}-{page}-{nonce}", std::process::id()));
    let output = Command::new(renderer)
        .arg("-png")
        .arg("-singlefile")
        .arg("-f")
        .arg(page.to_string())
        .arg("-l")
        .arg(page.to_string())
        .arg(path)
        .arg(&prefix)
        .output()
        .ok();
    let preview_path = prefix.with_extension("png");
    let preview = output.filter(|result| result.status.success()).and_then(|_| std::fs::read(&preview_path).ok());
    let _ = std::fs::remove_file(preview_path);
    preview.map(|bytes| format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

fn find_pdftoppm() -> Option<PathBuf> {
    std::env::var_os("TOOLBOX_PDFTOPPM_PATH")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty() && path.is_file())
        .or_else(|| Command::new("pdftoppm").arg("-h").output().ok().filter(|result| result.status.success()).map(|_| PathBuf::from("pdftoppm")))
}

pub(crate) fn page_bounds(document: &Document, page_id: lopdf::ObjectId) -> Result<(f32, f32, f32, f32), String> {
    let media_box = super::inherited(document, page_id, b"MediaBox")?
        .ok_or_else(|| "PDF page has no media box.".to_string())?;
    let values = media_box.as_array().map_err(|error| format!("PDF media box is invalid: {error}"))?;
    if values.len() != 4 {
        return Err("PDF media box must have four values.".to_string());
    }
    let left = number(&values[0])?;
    let bottom = number(&values[1])?;
    let right = number(&values[2])?;
    let top = number(&values[3])?;
    if right <= left || top <= bottom {
        return Err("PDF page has invalid dimensions.".to_string());
    }
    Ok((left, bottom, right, top))
}

pub(crate) fn number(value: &Object) -> Result<f32, String> {
    match value {
        Object::Integer(value) => Ok(*value as f32),
        Object::Real(value) => Ok(*value),
        _ => Err("PDF media box contains a non-numeric value.".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;

    #[test]
    fn reads_page_dimensions_without_normalizing_mixed_sizes() {
        let path = std::env::temp_dir().join(format!("toolbox_metadata_{}.pdf", std::process::id()));
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let mut page_ids = Vec::new();
        for (width, height) in [(612.0, 792.0), (792.0, 612.0)] {
            let page_id = document.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), width.into(), height.into()],
            });
            page_ids.push(Object::Reference(page_id));
        }
        document.objects.insert(pages_id, Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => page_ids,
            "Count" => 2,
        }));
        let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog_id);
        document.save(&path).unwrap();

        let metadata = inspect(&path).unwrap();
        assert_eq!(metadata.pages.len(), 2);
        assert_eq!((metadata.pages[0].x, metadata.pages[0].y), (0.0, 0.0));
        assert_eq!((metadata.pages[0].width, metadata.pages[0].height), (612.0, 792.0));
        assert_eq!(metadata.pages[0].rotation, 0);
        assert_eq!(metadata.pages[0].page_box, [0.0, 0.0, 612.0, 792.0]);
        assert_eq!((metadata.pages[1].width, metadata.pages[1].height), (792.0, 612.0));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn resolves_inherited_media_box_values() {
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let page_id = document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
        });
        document.objects.insert(pages_id, Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![Object::Reference(page_id)],
            "Count" => 1,
            "MediaBox" => vec![10.into(), 20.into(), 310.into(), 420.into()],
        }));

        assert_eq!(page_bounds(&document, page_id).unwrap(), (10.0, 20.0, 310.0, 420.0));
    }
}
