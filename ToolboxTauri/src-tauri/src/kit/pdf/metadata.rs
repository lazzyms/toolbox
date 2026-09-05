use lopdf::{Document, Object};
use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfPageMetadata {
    pub index: usize,
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
        .map(|(index, page_id)| page_bounds(&document, *page_id).map(|(left, bottom, right, top)| PdfPageMetadata { index, x: left, y: bottom, width: right - left, height: top - bottom }))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PdfDocumentMetadata { path: path.to_path_buf(), pages })
}

pub(crate) fn page_bounds(document: &Document, page_id: lopdf::ObjectId) -> Result<(f32, f32, f32, f32), String> {
    let page = document.get_dictionary(page_id).map_err(|error| format!("Could not read PDF page: {error}"))?;
    let media_box = page.get(b"MediaBox").map_err(|error| format!("PDF page has no media box: {error}"))?;
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
        assert_eq!((metadata.pages[0].width, metadata.pages[0].height), (612.0, 792.0));
        assert_eq!((metadata.pages[1].width, metadata.pages[1].height), (792.0, 612.0));
        let _ = std::fs::remove_file(path);
    }
}
