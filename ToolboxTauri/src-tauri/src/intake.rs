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

#[cfg(test)]
mod tests {
    use super::{inspect_intake_paths, PathKind};
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
}
