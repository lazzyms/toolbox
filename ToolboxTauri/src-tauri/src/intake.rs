use serde::Serialize;
use std::fs::File;
use std::path::{Path, PathBuf};

use crate::kit::images::compress::{png, webp};

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
    /// An animated PNG or WebP, which tools that take only still images flag before a run.
    pub animated: bool,
}

fn path_kind(path: &Path) -> PathKind {
    match path.metadata() {
        Ok(metadata) if metadata.is_dir() => PathKind::Folder,
        Ok(metadata) if metadata.is_file() && File::open(path).is_ok() => PathKind::File,
        _ => PathKind::Unreadable,
    }
}

fn is_animated(path: &Path) -> bool {
    let extension = path.extension().and_then(|extension| extension.to_str()).map(str::to_ascii_lowercase);
    let detect: fn(&[u8]) -> bool = match extension.as_deref() {
        Some("png") => png::is_animated,
        Some("webp") => webp::is_animated,
        _ => return false,
    };
    std::fs::read(path).is_ok_and(|bytes| detect(&bytes))
}

#[tauri::command]
pub fn inspect_intake_paths(paths: Vec<PathBuf>) -> Vec<InspectedPath> {
    paths
        .into_iter()
        .map(|path| {
            let kind = path_kind(&path);
            InspectedPath { animated: kind == PathKind::File && is_animated(&path), kind, path }
        })
        .collect()
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
    fn marks_animated_pngs_and_webps_and_leaves_still_and_other_files_unmarked() {
        let chunk = |name: &[u8; 4], data: &[u8]| [(data.len() as u32).to_be_bytes().as_slice(), name, data, &[0; 4]].concat();
        let png = |extra: &[Vec<u8>]| [&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A][..], &chunk(b"IHDR", &[0; 13]), &extra.concat(), &chunk(b"IEND", &[])].concat();
        let riff = |chunks: &[u8]| [b"RIFF".as_slice(), &(chunks.len() as u32 + 4).to_le_bytes(), b"WEBP", chunks].concat();
        let webp_chunk = |name: &[u8; 4], data: &[u8]| [name.as_slice(), &(data.len() as u32).to_le_bytes(), data].concat();
        let files = [
            ("animated.png", png(&[chunk(b"acTL", &[0; 8])]), true),
            ("still.png", png(&[]), false),
            ("animated.WEBP", riff(&webp_chunk(b"VP8X", &[0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0])), true),
            ("still.webp", riff(&webp_chunk(b"VP8L", &[0x2f, 0, 0, 0, 0])), false),
            ("animated-named.gif", png(&[chunk(b"acTL", &[0; 8])]), false),
        ];
        let paths = files.iter().map(|(name, bytes, _)| {
            let path = unique_path(name);
            std::fs::write(&path, bytes).unwrap();
            path
        }).collect::<Vec<_>>();

        let inspected = inspect_intake_paths(paths.clone());

        assert_eq!(inspected.iter().map(|item| item.animated).collect::<Vec<_>>(), files.iter().map(|(_, _, animated)| *animated).collect::<Vec<_>>());
        assert_eq!(serde_json::to_value(&inspected[0]).unwrap()["animated"], true);
        for ((_, bytes, _), path) in files.iter().zip(&paths) {
            assert_eq!(&std::fs::read(path).unwrap(), bytes, "inspection must not change the file");
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn serializes_kinds_for_the_intake_list() {
        let inspected = inspect_intake_paths(vec![unique_path("absent.png")]);
        assert_eq!(serde_json::to_value(&inspected).unwrap()[0]["kind"], "unreadable");
    }
}
