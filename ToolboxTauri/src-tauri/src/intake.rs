use serde::Serialize;
use std::fs::File;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PathKind {
    File,
    Folder,
    Unreadable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectedPath {
    pub path: PathBuf,
    pub kind: PathKind,
}

fn path_kind(path: &Path) -> PathKind {
    match path.metadata() {
        Ok(metadata) if metadata.is_dir() => PathKind::Folder,
        Ok(metadata) if metadata.is_file() && File::open(path).is_ok() => PathKind::File,
        _ => PathKind::Unreadable,
    }
}

#[tauri::command]
pub fn inspect_intake_paths(paths: Vec<PathBuf>) -> Vec<InspectedPath> {
    paths.into_iter().map(|path| InspectedPath { kind: path_kind(&path), path }).collect()
}

/// What a file list shows for each entry. A fact that can't be read is left out, so the list still shows the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileFacts {
    pub path: PathBuf,
    pub size_bytes: Option<u64>,
    pub modified_ms: Option<u64>,
    pub page_count: Option<u32>,
}

fn file_facts(path: PathBuf) -> FileFacts {
    let metadata = path.metadata().ok().filter(|metadata| metadata.is_file());
    let modified_ms = metadata.as_ref()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|age| age.as_millis() as u64);
    let is_pdf = path.extension().and_then(|extension| extension.to_str()).is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"));
    // Only the trailer and page tree are read, so a long list stays quick and the file is never changed.
    let page_count = (metadata.is_some() && is_pdf).then(|| lopdf::Document::load_metadata(&path).ok().map(|facts| facts.page_count)).flatten();
    FileFacts { size_bytes: metadata.map(|metadata| metadata.len()), modified_ms, page_count, path }
}

#[tauri::command]
pub async fn describe_intake_files(paths: Vec<PathBuf>) -> Vec<FileFacts> {
    tauri::async_runtime::spawn_blocking(move || paths.into_iter().map(file_facts).collect())
        .await
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{describe_intake_files, inspect_intake_paths, PathKind};
    use std::path::PathBuf;

    fn unique_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("toolbox_intake_{}_{}", std::process::id(), name))
    }

    #[test]
    fn classifies_each_dropped_path_independently() {
        let file = unique_path("report.pdf");
        let folder = unique_path("scans.pdf");
        let missing = unique_path("missing.pdf");
        std::fs::write(&file, b"%PDF-1.7").unwrap();
        std::fs::create_dir_all(&folder).unwrap();

        let inspected = inspect_intake_paths(vec![folder.clone(), missing.clone(), file.clone()]);

        assert_eq!(
            inspected.iter().map(|item| (item.path.clone(), item.kind)).collect::<Vec<_>>(),
            vec![(folder.clone(), PathKind::Folder), (missing, PathKind::Unreadable), (file.clone(), PathKind::File)]
        );
        assert_eq!(std::fs::read(&file).unwrap(), b"%PDF-1.7");
        std::fs::remove_file(file).unwrap();
        std::fs::remove_dir(folder).unwrap();
    }

    #[test]
    fn serializes_kinds_for_the_intake_list() {
        let inspected = inspect_intake_paths(vec![unique_path("absent.png")]);
        assert_eq!(serde_json::to_value(&inspected).unwrap()[0]["kind"], "unreadable");
    }

    #[test]
    fn describes_page_count_size_and_date_without_touching_the_file() {
        use lopdf::{dictionary, Document, Object, Stream};
        let pdf = unique_path("facts.pdf");
        let damaged = unique_path("damaged.pdf");
        let missing = unique_path("gone.pdf");
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let kids = (0..3).map(|number| {
            let content = document.add_object(Stream::new(dictionary! {}, format!("BT ({number}) Tj ET").into_bytes()));
            Object::Reference(document.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()], "Contents" => content }))
        }).collect::<Vec<_>>();
        document.objects.insert(pages_id, dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => 3 }.into());
        let catalog = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog);
        document.save(&pdf).unwrap();
        std::fs::write(&damaged, b"%PDF-1.7 not really").unwrap();
        let before = std::fs::read(&pdf).unwrap();

        let facts = tauri::async_runtime::block_on(describe_intake_files(vec![pdf.clone(), damaged.clone(), missing.clone(), pdf.clone()]));

        assert_eq!(facts.iter().map(|facts| facts.path.clone()).collect::<Vec<_>>(), vec![pdf.clone(), damaged.clone(), missing.clone(), pdf.clone()]);
        assert_eq!(facts.iter().map(|facts| facts.page_count).collect::<Vec<_>>(), vec![Some(3), None, None, Some(3)]);
        assert_eq!(facts[0].size_bytes, Some(before.len() as u64));
        assert!(facts[0].modified_ms.is_some_and(|modified| modified > 0));
        assert_eq!((facts[2].size_bytes, facts[2].modified_ms), (None, None));
        assert_eq!(serde_json::to_value(&facts[0]).unwrap()["pageCount"], 3);
        assert_eq!(std::fs::read(&pdf).unwrap(), before);
        std::fs::remove_file(pdf).unwrap();
        std::fs::remove_file(damaged).unwrap();
    }
}
