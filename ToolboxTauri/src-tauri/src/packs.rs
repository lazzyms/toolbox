//! Optional packs: the built-in catalog and what is installed in the app-data folder.
//!
//! Listing is read-only and local. It reads the catalog built into the app and the files under
//! `<app data>/packs`, and never touches the network. Only [`download`] does. [`uninstall`] removes
//! a pack unless a run is using it. [`health`] checks a pack's files at its first use in a session.
//!
//! Installed layout, one folder per pack, in the per-user local app-data folder (non-roaming on
//! Windows):
//! - `packs/<id>/installed.json` — `{ "version", "packFormat", "appVersions": { "from", "before"? },
//!   "files": { "<relative path>": <bytes> }, "manifestSha256" }`, written last by [`install`].
//! - `packs/<id>/<version>/manifest.json` — the signed pack manifest, a
//!   [`crate::kit::resources::ResourceManifest`] that discovery reads as the installed pack root.

pub mod download;
pub mod health;
pub mod install;
pub mod uninstall;

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::path::{Component, Path, PathBuf};

/// The newest pack format this build can read.
pub const PACK_FORMAT: u32 = 1;
/// The catalog format this build can read.
const CATALOG_FORMAT: u32 = 1;
/// Payloads for this target serve every Supported target. Data-only packs use it.
const ALL_TARGETS: &str = "all";
const BUILT_IN_CATALOG: &str = include_str!("../../shared/pack-catalog.json");
/// Development override: a path to a fixture catalog. Release builds ignore it.
#[cfg(debug_assertions)]
const CATALOG_OVERRIDE_ENV: &str = "TOOLBOX_PACK_CATALOG";
const PACKS_DIR: &str = "packs";
const INSTALLED_FILE: &str = "installed.json";
const MANIFEST_FILE: &str = "manifest.json";

/// The per-user app-data folder that holds `packs/`. Local, so packs never roam between Windows
/// computers; on macOS this is `~/Library/Application Support/<identifier>`.
pub fn app_data(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    use tauri::Manager;
    app.path().app_local_data_dir().map_err(|error| error.to_string())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub format_version: u32,
    pub packs: Vec<CatalogPack>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogPack {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tool_ids: Vec<String>,
    pub license: String,
    pub version: String,
    pub min_pack_format: u32,
    pub app_versions: VersionRange,
    pub payloads: Vec<Payload>,
    /// The language a text recognition pack adds. OCR finds the pack a language needs by its code.
    #[serde(default)]
    pub ocr_language: Option<OcrLanguage>,
    /// The fonts a script pack adds: a Unicode script's full name, as in `Devanagari`, and the
    /// TrueType font file in the pack that writes it. Edit PDF finds the pack a script needs here.
    #[serde(default)]
    pub script_fonts: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrLanguage {
    /// The language's Tesseract code, as in `hin`. The pack holds `tessdata/<code>.traineddata`.
    pub code: String,
    /// How results name the pack, as in "Install Hindi pack".
    pub name: String,
}

/// The tool whose language packs the catalog names with `ocrLanguage`.
pub const OCR_TOOL_ID: &str = "pdf-ocr";
/// The text recognition language built into the app, as its Tesseract code and name. Every other
/// language comes from a pack.
pub const BUILT_IN_OCR_LANGUAGE: (&str, &str) = ("eng", "English");
/// The tool whose script packs the catalog names with `scriptFonts`.
pub const PDF_TEXT_TOOL_ID: &str = "pdf-edit";

/// `from` is inclusive and `before` is exclusive. No `before` means every later version.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionRange {
    pub from: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Payload {
    /// `all`, or a Supported target such as `macos-aarch64`.
    pub target: String,
    pub download_bytes: u64,
    pub installed_bytes: u64,
    /// Lowercase hex SHA-256 of the signed payload archive.
    pub sha256: String,
    /// Where the pack file is on the release server, relative to the server's download base:
    /// plain path segments joined by `/`, ending in `.toolboxpack`.
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PackStatus {
    NotInstalled,
    Installed,
    UpdateAvailable,
    Damaged,
    Incompatible,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tool_ids: Vec<String>,
    pub license: String,
    /// The version the catalog offers.
    pub version: String,
    /// Download and installed size of this target's payload. Absent when the catalog has no
    /// payload this build can install.
    pub download_bytes: Option<u64>,
    pub installed_bytes: Option<u64>,
    /// What the pack's folder uses on this disk now.
    pub disk_bytes: u64,
    pub status: PackStatus,
    pub installed_version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackList {
    pub packs: Vec<PackEntry>,
    pub installed_count: usize,
    /// Everything under the packs folder, including leftovers no catalog entry names.
    pub total_bytes: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstalledRecord {
    pub(crate) version: String,
    pub(crate) pack_format: u32,
    pub(crate) app_versions: VersionRange,
    /// Every file in the version folder with its exact size, as verified at install.
    pub(crate) files: std::collections::BTreeMap<String, u64>,
    /// SHA-256 of the signed manifest, so a changed manifest is found at first use. Absent in
    /// records written before it was added.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) manifest_sha256: Option<String>,
}

impl InstalledRecord {
    /// Whether this build can use the installed pack: its format and app range fit. Tools never
    /// load a pack that does not fit, and listing marks it incompatible.
    fn fits(&self, app_version: &str) -> bool {
        self.pack_format <= PACK_FORMAT && in_range(app_version, &self.app_versions)
    }
}

/// The installed record in the pack folder `folder`, when it can be read.
fn read_record(folder: &Path) -> Option<InstalledRecord> {
    serde_json::from_str(&std::fs::read_to_string(folder.join(INSTALLED_FILE)).ok()?).ok()
}

/// A language OCR reads, or can read once its pack is installed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrLanguageState {
    /// The Tesseract code an OCR request names, as in `hin`.
    pub code: String,
    pub name: String,
    /// The pack that adds the language. `None` for the language built into the app.
    pub pack_id: Option<String>,
    pub status: PackStatus,
    /// The download its pack needs on this target, when the catalog offers one.
    pub download_bytes: Option<u64>,
}

#[tauri::command]
pub async fn ocr_languages(app: tauri::AppHandle) -> Result<Vec<OcrLanguageState>, String> {
    let catalog = load_catalog()?;
    let app_data = app_data(&app)?;
    Ok(ocr_languages_in(&catalog, &app_data, &current_target(), &app.package_info().version.to_string()))
}

#[tauri::command]
pub async fn list_packs(app: tauri::AppHandle) -> Result<PackList, String> {
    let catalog = load_catalog()?;
    let app_data = app_data(&app)?;
    Ok(list_packs_in(&catalog, &app_data, &current_target(), &app.package_info().version.to_string()))
}

pub fn current_target() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

pub fn load_catalog() -> Result<Catalog, String> {
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os(CATALOG_OVERRIDE_ENV) {
        let text = std::fs::read_to_string(&path).map_err(|error| format!("Could not read the pack catalog: {error}"))?;
        return parse_catalog(&text);
    }
    parse_catalog(BUILT_IN_CATALOG)
}

pub fn parse_catalog(text: &str) -> Result<Catalog, String> {
    let catalog: Catalog = serde_json::from_str(text).map_err(|error| format!("The pack catalog is not valid: {error}"))?;
    if catalog.format_version != CATALOG_FORMAT {
        return Err(format!("The pack catalog format {} is not supported", catalog.format_version));
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut languages = std::collections::BTreeSet::from([BUILT_IN_OCR_LANGUAGE.0]);
    let mut scripts = std::collections::BTreeSet::new();
    for pack in &catalog.packs {
        let valid_id = !pack.id.is_empty() && pack.id.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
        if !valid_id || !ids.insert(pack.id.as_str()) {
            return Err(format!("The pack catalog has a bad or repeated id: {:?}", pack.id));
        }
        for version in [Some(&pack.version), Some(&pack.app_versions.from), pack.app_versions.before.as_ref()].into_iter().flatten() {
            if parse_version(version).is_none() {
                return Err(format!("Pack {} has a bad version: {version:?}", pack.id));
            }
        }
        // The version names the installed folder, so it must be one plain path segment.
        if !install::is_plain_segment(&pack.version) {
            return Err(format!("Pack {} has a bad version: {:?}", pack.id, pack.version));
        }
        let valid_sha = |sha: &str| sha.len() == 64 && sha.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if let Some(payload) = pack.payloads.iter().find(|payload| !valid_sha(&payload.sha256)) {
            return Err(format!("Pack {} has a bad checksum for {}", pack.id, payload.target));
        }
        // Download locations come only from this catalog, so each must stay under the server's base.
        let valid_path = |path: &str| path.ends_with(".toolboxpack") && path.split('/').all(install::is_plain_segment);
        if let Some(payload) = pack.payloads.iter().find(|payload| !valid_path(&payload.path)) {
            return Err(format!("Pack {} has a bad download path for {}: {:?}", pack.id, payload.target, payload.path));
        }
        if let Some(language) = &pack.ocr_language {
            // The code names a file in the pack, so it must be one plain Tesseract language name.
            let valid_code = !language.code.is_empty() && language.code.bytes().all(|byte| byte.is_ascii_lowercase() || byte == b'_');
            if !valid_code || language.name.trim().is_empty() || !pack.tool_ids.iter().any(|tool| tool == OCR_TOOL_ID) {
                return Err(format!("Pack {} has a bad text recognition language", pack.id));
            }
            if !languages.insert(language.code.as_str()) {
                return Err(format!("The pack catalog has more than one pack for language {:?}", language.code));
            }
        }
        if !pack.script_fonts.is_empty() && !pack.tool_ids.iter().any(|tool| tool == PDF_TEXT_TOOL_ID) {
            return Err(format!("Pack {} adds script fonts but not to {PDF_TEXT_TOOL_ID}", pack.id));
        }
        for (script, font) in &pack.script_fonts {
            // Scripts the app writes with its own font never come from a pack.
            if crate::kit::pdf::script_text::pack_script(script).is_none() {
                return Err(format!("Pack {} has a bad script: {script:?}", pack.id));
            }
            // The font names a file in the pack, so it must stay inside it.
            if !font.ends_with(".ttf") || !font.split('/').all(install::is_plain_segment) {
                return Err(format!("Pack {} has a bad font for {script}: {font:?}", pack.id));
            }
            if !scripts.insert(script.as_str()) {
                return Err(format!("The pack catalog has more than one pack for script {script:?}"));
            }
        }
    }
    Ok(catalog)
}

/// Merges the catalog with what is installed under `app_data`. Packs that cannot be installed on
/// `target` or with `app_version` are left out unless they are already installed.
pub fn list_packs_in(catalog: &Catalog, app_data: &Path, target: &str, app_version: &str) -> PackList {
    let packs_root = app_data.join(PACKS_DIR);
    let packs: Vec<PackEntry> = catalog.packs.iter().filter_map(|pack| entry_for(pack, &packs_root.join(&pack.id), target, app_version)).collect();
    PackList {
        installed_count: packs.iter().filter(|pack| pack.status != PackStatus::NotInstalled).count(),
        total_bytes: disk_usage(&packs_root),
        packs,
    }
}

/// The pack-state query behind the OCR language picker: the built-in language, then each language
/// whose pack the catalog offers on `target` or is installed, with the pack's state. Read-only and
/// local like [`list_packs_in`]; a run still checks the pack's files at first use.
pub fn ocr_languages_in(catalog: &Catalog, app_data: &Path, target: &str, app_version: &str) -> Vec<OcrLanguageState> {
    let (code, name) = BUILT_IN_OCR_LANGUAGE;
    let built_in = OcrLanguageState { code: code.into(), name: name.into(), pack_id: None, status: PackStatus::Installed, download_bytes: None };
    let packs_root = app_data.join(PACKS_DIR);
    let packs = catalog.packs.iter().filter_map(|pack| {
        let language = pack.ocr_language.as_ref()?;
        let entry = entry_for(pack, &packs_root.join(&pack.id), target, app_version)?;
        Some(OcrLanguageState { code: language.code.clone(), name: language.name.clone(), pack_id: Some(entry.id), status: entry.status, download_bytes: entry.download_bytes })
    });
    std::iter::once(built_in).chain(packs).collect()
}

fn entry_for(pack: &CatalogPack, folder: &Path, target: &str, app_version: &str) -> Option<PackEntry> {
    let payload = offered_payload(pack, target, app_version);
    let (status, installed_version) = installed_state(pack, folder, app_version, payload.is_some());
    if payload.is_none() && status == PackStatus::NotInstalled {
        return None;
    }
    Some(PackEntry {
        id: pack.id.clone(),
        name: pack.name.clone(),
        description: pack.description.clone(),
        tool_ids: pack.tool_ids.clone(),
        license: pack.license.clone(),
        version: pack.version.clone(),
        download_bytes: payload.map(|payload| payload.download_bytes),
        installed_bytes: payload.map(|payload| payload.installed_bytes),
        disk_bytes: if status == PackStatus::NotInstalled { 0 } else { disk_usage(folder) },
        status,
        installed_version,
    })
}

/// A payload built for this target wins over a data-only one.
fn offered_payload<'a>(pack: &'a CatalogPack, target: &str, app_version: &str) -> Option<&'a Payload> {
    if pack.min_pack_format > PACK_FORMAT || !in_range(app_version, &pack.app_versions) {
        return None;
    }
    pack.payloads.iter().find(|payload| payload.target == target)
        .or_else(|| pack.payloads.iter().find(|payload| payload.target == ALL_TARGETS))
}

fn installed_state(pack: &CatalogPack, folder: &Path, app_version: &str, offered: bool) -> (PackStatus, Option<String>) {
    let record_path = folder.join(INSTALLED_FILE);
    if !record_path.exists() {
        return (PackStatus::NotInstalled, None);
    }
    let Some(record) = read_record(folder).filter(|record| parse_version(&record.version).is_some()) else {
        return (PackStatus::Damaged, None);
    };
    let version = Some(record.version.clone());
    if health::known_damaged(folder) || !files_intact(&folder.join(&record.version), &record.files) {
        return (PackStatus::Damaged, version);
    }
    let newer_offered = offered && compare_versions(&pack.version, &record.version) == Some(Ordering::Greater);
    if newer_offered {
        return (PackStatus::UpdateAvailable, version);
    }
    if !record.fits(app_version) {
        return (PackStatus::Incompatible, version);
    }
    (PackStatus::Installed, version)
}

/// The version folder still has its manifest and every recorded file with its recorded size.
/// Listing does not hash files; checksums are verified at install and at the pack's first use in
/// a session ([`health`]), and listing reports a pack that check found damaged.
fn files_intact(version_folder: &Path, files: &std::collections::BTreeMap<String, u64>) -> bool {
    files.contains_key(MANIFEST_FILE) && files.iter().all(|(relative, bytes)| {
        let relative = Path::new(relative);
        let inside = relative.components().all(|component| matches!(component, Component::Normal(_)));
        inside && std::fs::symlink_metadata(version_folder.join(relative))
            .is_ok_and(|metadata| metadata.is_file() && metadata.len() == *bytes)
    })
}

/// Recursive size of regular files under `path`. Symbolic links are not followed.
fn disk_usage(path: &Path) -> u64 {
    let mut total = 0;
    let mut pending: Vec<PathBuf> = vec![path.to_path_buf()];
    while let Some(next) = pending.pop() {
        let Ok(metadata) = std::fs::symlink_metadata(&next) else { continue };
        if metadata.is_file() {
            total += metadata.len();
        } else if metadata.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&next) {
                pending.extend(entries.flatten().map(|entry| entry.path()));
            }
        }
    }
    total
}

/// The folder discovery reads as the installed pack root for `id`: the recorded version's folder.
/// `None` when nothing is recorded. Discovery verifies every file it uses against the manifest.
#[cfg_attr(not(test), allow(dead_code))]
pub fn installed_pack_root(app_data: &Path, id: &str) -> Option<PathBuf> {
    let folder = app_data.join(PACKS_DIR).join(id);
    let record = read_record(&folder)?;
    install::is_plain_segment(&record.version).then(|| folder.join(record.version))
}

fn in_range(version: &str, range: &VersionRange) -> bool {
    compare_versions(version, &range.from).is_some_and(|order| order != Ordering::Less)
        && range.before.as_deref().is_none_or(|before| compare_versions(version, before) == Some(Ordering::Less))
}

/// Dotted numeric versions. Pre-release and build suffixes (`-beta`, `+abc`) are ignored.
fn parse_version(version: &str) -> Option<Vec<u64>> {
    let core = version.split(['-', '+']).next()?;
    core.split('.').map(|part| part.parse().ok()).collect()
}

fn compare_versions(left: &str, right: &str) -> Option<Ordering> {
    let (mut left, mut right) = (parse_version(left)?, parse_version(right)?);
    let length = left.len().max(right.len());
    left.resize(length, 0);
    right.resize(length, 0);
    Some(left.cmp(&right))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE_CATALOG: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/fixtures/pack-catalog.json");
    const MAC: &str = "macos-aarch64";
    const APP: &str = "1.0.14";
    const MANIFEST_JSON: &str = r#"{"version":1,"architecture":"all","resources":{}}"#;

    fn sandbox(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("toolbox-packs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn fixture() -> Catalog {
        parse_catalog(&std::fs::read_to_string(FIXTURE_CATALOG).unwrap()).unwrap()
    }

    /// Writes a complete install of `version` with one data file and returns its folder. The
    /// record gains the file sizes unless it already names them.
    fn install(app_data: &Path, id: &str, version: &str, mut record: serde_json::Value) -> PathBuf {
        let folder = app_data.join(PACKS_DIR).join(id);
        let version_folder = folder.join(version).join("data");
        std::fs::create_dir_all(&version_folder).unwrap();
        std::fs::write(version_folder.join("payload.bin"), vec![7u8; 1000]).unwrap();
        std::fs::write(folder.join(version).join(MANIFEST_FILE), MANIFEST_JSON).unwrap();
        if let Some(fields) = record.as_object_mut() {
            fields.entry("files").or_insert(serde_json::json!({ "data/payload.bin": 1000, "manifest.json": MANIFEST_JSON.len() }));
        }
        std::fs::write(folder.join(INSTALLED_FILE), record.to_string()).unwrap();
        folder
    }

    fn record(version: &str) -> serde_json::Value {
        serde_json::json!({ "version": version, "packFormat": 1, "appVersions": { "from": "0.1.0" } })
    }

    fn record_size(version: &str) -> u64 {
        let mut value = record(version);
        value.as_object_mut().unwrap().insert("files".into(), serde_json::json!({ "data/payload.bin": 1000, "manifest.json": MANIFEST_JSON.len() }));
        value.to_string().len() as u64
    }

    fn find<'a>(list: &'a PackList, id: &str) -> Option<&'a PackEntry> {
        list.packs.iter().find(|pack| pack.id == id)
    }

    #[test]
    fn built_in_catalog_parses() {
        let catalog = parse_catalog(BUILT_IN_CATALOG).unwrap();
        assert!(catalog.packs.iter().all(|pack| !pack.payloads.is_empty()));
    }

    #[test]
    fn fixture_catalog_is_filtered_to_the_target_and_merged_with_installed_state_and_disk_usage() {
        let app_data = sandbox("merge");
        install(&app_data, "unicode-scripts", "1.0", record("1.0"));
        install(&app_data, "ocr-german", "4.0", record("4.0"));
        let list = list_packs_in(&fixture(), &app_data, MAC, APP);

        let ids: Vec<_> = list.packs.iter().map(|pack| pack.id.as_str()).collect();
        assert_eq!(ids, ["unicode-scripts", "ocr-german", "ocr-french", "ocr-japanese"]);

        let unicode = find(&list, "unicode-scripts").unwrap();
        assert_eq!((unicode.status, unicode.installed_version.as_deref()), (PackStatus::Installed, Some("1.0")));
        assert_eq!(unicode.disk_bytes, 1000 + MANIFEST_JSON.len() as u64 + record_size("1.0"));
        assert_eq!(unicode.download_bytes, Some(52428800));

        let german = find(&list, "ocr-german").unwrap();
        assert_eq!((german.status, german.installed_version.as_deref()), (PackStatus::UpdateAvailable, Some("4.0")));

        let french = find(&list, "ocr-french").unwrap();
        assert_eq!((french.status, french.disk_bytes, french.installed_version.as_deref()), (PackStatus::NotInstalled, 0, None));

        // A native payload for this target is chosen.
        assert_eq!(find(&list, "ocr-japanese").unwrap().installed_bytes, Some(31457280));
        assert_eq!(list.installed_count, 2);
        assert_eq!(list.total_bytes, unicode.disk_bytes + german.disk_bytes);
    }

    #[test]
    fn a_pack_whose_only_payloads_are_native_for_other_targets_is_not_offered() {
        let app_data = sandbox("other-target");
        let catalog = fixture();
        assert!(list_packs_in(&catalog, &app_data, MAC, APP).packs.iter().all(|pack| pack.id != "other-target-only"));
        assert!(list_packs_in(&catalog, &app_data, "windows-x86_64", APP).packs.iter().all(|pack| pack.id != "other-target-only"));
        let windows_arm = list_packs_in(&catalog, &app_data, "windows-aarch64", APP);
        assert_eq!(find(&windows_arm, "other-target-only").unwrap().status, PackStatus::NotInstalled);
        // An unsupported target gets data-only packs and nothing native.
        let ids: Vec<_> = list_packs_in(&catalog, &app_data, "linux-x86_64", APP).packs.into_iter().map(|pack| pack.id).collect();
        assert_eq!(ids, ["unicode-scripts", "ocr-german", "ocr-french"]);
    }

    #[test]
    fn missing_or_resized_files_and_unreadable_records_are_damaged() {
        let app_data = sandbox("damaged");
        let unicode = install(&app_data, "unicode-scripts", "1.0", record("1.0"));
        std::fs::remove_file(unicode.join("1.0/data/payload.bin")).unwrap();
        let german = install(&app_data, "ocr-german", "4.1", record("4.1"));
        std::fs::write(german.join("4.1/data/payload.bin"), b"short").unwrap();
        let french = install(&app_data, "ocr-french", "4.1", record("4.1"));
        std::fs::write(french.join(INSTALLED_FILE), "not json").unwrap();
        let mut escaping = record("4.1");
        escaping.as_object_mut().unwrap().insert("files".into(), serde_json::json!({ "manifest.json": MANIFEST_JSON.len(), "../installed.json": 10 }));
        install(&app_data, "ocr-japanese", "4.1", escaping);
        let mut unlisted_manifest = record("1.0");
        unlisted_manifest.as_object_mut().unwrap().insert("files".into(), serde_json::json!({ "data/payload.bin": 1000 }));
        install(&app_data, "other-target-only", "1.0", unlisted_manifest);

        let list = list_packs_in(&fixture(), &app_data, MAC, APP);
        for id in ["unicode-scripts", "ocr-german", "ocr-french", "ocr-japanese", "other-target-only"] {
            assert_eq!(find(&list, id).unwrap().status, PackStatus::Damaged, "{id}");
        }
        assert_eq!(find(&list, "ocr-french").unwrap().installed_version, None);
        assert_eq!(find(&list, "unicode-scripts").unwrap().installed_version.as_deref(), Some("1.0"));
    }

    #[test]
    fn installs_for_a_newer_pack_format_or_another_app_version_are_incompatible() {
        let app_data = sandbox("incompatible");
        install(&app_data, "unicode-scripts", "1.0", serde_json::json!({ "version": "1.0", "packFormat": 2, "appVersions": { "from": "0.1.0" } }));
        install(&app_data, "ocr-french", "4.1", serde_json::json!({ "version": "4.1", "packFormat": 1, "appVersions": { "from": "0.1.0", "before": "1.0.0" } }));
        // Installed for another target: still listed, judged on its own record, never offered an update.
        install(&app_data, "other-target-only", "0.9", serde_json::json!({ "version": "0.9", "packFormat": 1, "appVersions": { "from": "2.0.0" } }));
        let list = list_packs_in(&fixture(), &app_data, MAC, APP);
        assert_eq!(find(&list, "unicode-scripts").unwrap().status, PackStatus::Incompatible);
        assert_eq!(find(&list, "ocr-french").unwrap().status, PackStatus::Incompatible);
        let other = find(&list, "other-target-only").unwrap();
        assert_eq!((other.status, other.download_bytes), (PackStatus::Incompatible, None));
    }

    #[test]
    fn an_incompatible_install_with_a_newer_catalog_version_offers_the_update() {
        let app_data = sandbox("incompatible-update");
        install(&app_data, "ocr-german", "4.0", serde_json::json!({ "version": "4.0", "packFormat": 1, "appVersions": { "from": "0.1.0", "before": "1.0.0" } }));
        assert_eq!(find(&list_packs_in(&fixture(), &app_data, MAC, APP), "ocr-german").unwrap().status, PackStatus::UpdateAvailable);
    }

    #[test]
    fn catalog_entries_for_a_newer_pack_format_or_app_are_not_offered() {
        let mut catalog = fixture();
        catalog.packs[0].min_pack_format = PACK_FORMAT + 1;
        catalog.packs[1].app_versions = VersionRange { from: "9.0".into(), before: None };
        catalog.packs[2].app_versions = VersionRange { from: "0.1".into(), before: Some("1.0.14".into()) };
        let list = list_packs_in(&catalog, &sandbox("not-offered"), MAC, APP);
        let ids: Vec<_> = list.packs.iter().map(|pack| pack.id.as_str()).collect();
        assert_eq!(ids, ["ocr-japanese"]);
    }

    #[test]
    fn total_disk_usage_counts_leftovers_and_an_empty_folder_uses_nothing() {
        let app_data = sandbox("total");
        assert_eq!(list_packs_in(&fixture(), &app_data, MAC, APP).total_bytes, 0);
        std::fs::create_dir_all(app_data.join(PACKS_DIR).join("retired-pack")).unwrap();
        std::fs::write(app_data.join(PACKS_DIR).join("retired-pack/file"), vec![0u8; 321]).unwrap();
        let list = list_packs_in(&fixture(), &app_data, MAC, APP);
        assert_eq!((list.total_bytes, list.installed_count), (321, 0));
    }

    #[cfg(debug_assertions)]
    #[test]
    fn the_development_override_supplies_the_fixture_catalog() {
        let _guard = crate::kit::lock_process_env();
        std::env::set_var(CATALOG_OVERRIDE_ENV, FIXTURE_CATALOG);
        let overridden = load_catalog();
        std::env::remove_var(CATALOG_OVERRIDE_ENV);
        assert_eq!(overridden.unwrap().packs.len(), 5);
        assert_eq!(load_catalog().unwrap().packs.len(), parse_catalog(BUILT_IN_CATALOG).unwrap().packs.len());
    }

    #[test]
    fn bad_catalogs_are_rejected() {
        assert!(parse_catalog(r#"{"formatVersion":2,"packs":[]}"#).is_err());
        let pack = |id: &str, version: &str| serde_json::json!({
            "id": id, "name": "n", "description": "d", "toolIds": [], "license": "l", "version": version,
            "minPackFormat": 1, "appVersions": { "from": "0.1" }, "payloads": []
        });
        let mut bad_checksum = pack("a", "1");
        bad_checksum["payloads"] = serde_json::json!([{ "target": "all", "downloadBytes": 1, "installedBytes": 1, "sha256": "ABC", "path": "a.toolboxpack" }]);
        let with_path = |path: &str| {
            let mut pack = pack("a", "1");
            pack["payloads"] = serde_json::json!([{ "target": "all", "downloadBytes": 1, "installedBytes": 1, "sha256": "0".repeat(64), "path": path }]);
            pack
        };
        let bad_paths = ["../a.toolboxpack", "/a.toolboxpack", "a/../b.toolboxpack", "https://elsewhere.example/a.toolboxpack", "a.toolboxpack?x=1", "a//b.toolboxpack", "a.zip", ""].map(|path| vec![with_path(path)]);
        assert!(parse_catalog(&serde_json::json!({ "formatVersion": 1, "packs": [with_path("packs/a-1/a-1-all.toolboxpack")] }).to_string()).is_ok());
        for packs in [vec![pack("../escape", "1")], vec![pack("a", "1"), pack("a", "1")], vec![pack("a", "one")], vec![pack("a", "1-/../x")], vec![bad_checksum]].into_iter().chain(bad_paths) {
            assert!(parse_catalog(&serde_json::json!({ "formatVersion": 1, "packs": packs }).to_string()).is_err());
        }
    }

    #[test]
    fn versions_compare_numerically() {
        assert_eq!(compare_versions("3.10", "3.9"), Some(Ordering::Greater));
        assert_eq!(compare_versions("1.0", "1.0.0"), Some(Ordering::Equal));
        assert_eq!(compare_versions("1.0.14-beta.1", "1.0.14"), Some(Ordering::Equal));
        assert!(in_range("1.0.14", &VersionRange { from: "1.0.14".into(), before: Some("1.1".into()) }));
        assert!(!in_range("1.1.0", &VersionRange { from: "1.0".into(), before: Some("1.1".into()) }));
    }

    fn ocr_states(list: &[OcrLanguageState]) -> Vec<(&str, Option<&str>, PackStatus, Option<u64>)> {
        list.iter().map(|language| (language.code.as_str(), language.pack_id.as_deref(), language.status, language.download_bytes)).collect()
    }

    #[test]
    fn english_is_the_only_ocr_language_with_the_built_in_catalog_and_no_packs() {
        let list = ocr_languages_in(&parse_catalog(BUILT_IN_CATALOG).unwrap(), &sandbox("ocr-built-in"), MAC, APP);
        assert_eq!(list, [OcrLanguageState { code: "eng".into(), name: "English".into(), pack_id: None, status: PackStatus::Installed, download_bytes: None }]);
    }

    #[test]
    fn the_ocr_language_query_reports_each_pack_state_from_the_catalog_and_the_pack_folder() {
        let catalog = fixture();
        let nothing_installed = ocr_languages_in(&catalog, &sandbox("ocr-none"), MAC, APP);
        assert_eq!(ocr_states(&nothing_installed), [
            ("eng", None, PackStatus::Installed, None),
            ("deu", Some("ocr-german"), PackStatus::NotInstalled, Some(15728640)),
            ("fra", Some("ocr-french"), PackStatus::NotInstalled, Some(14680064)),
            ("jpn", Some("ocr-japanese"), PackStatus::NotInstalled, Some(20971520)),
        ]);
        assert_eq!(nothing_installed.iter().map(|language| language.name.as_str()).collect::<Vec<_>>(), ["English", "German", "French", "Japanese"]);

        let app_data = sandbox("ocr-states");
        install(&app_data, "ocr-german", "4.1", record("4.1"));
        let french = install(&app_data, "ocr-french", "4.1", record("4.1"));
        std::fs::write(french.join("4.1/data/payload.bin"), b"changed").unwrap();
        install(&app_data, "ocr-japanese", "4.1", serde_json::json!({ "version": "4.1", "packFormat": 1, "appVersions": { "from": "0.1.0", "before": "1.0.0" } }));
        assert_eq!(ocr_states(&ocr_languages_in(&catalog, &app_data, MAC, APP)), [
            ("eng", None, PackStatus::Installed, None),
            ("deu", Some("ocr-german"), PackStatus::Installed, Some(15728640)),
            ("fra", Some("ocr-french"), PackStatus::Damaged, Some(14680064)),
            ("jpn", Some("ocr-japanese"), PackStatus::Incompatible, Some(20971520)),
        ]);
        // A language whose pack this target can't install is left out, as in the pack list.
        assert!(ocr_languages_in(&catalog, &sandbox("ocr-linux"), "linux-x86_64", APP).iter().all(|language| language.code != "jpn"));
    }

    const LAUNCH_OCR_LANGUAGES: &str = include_str!("../../shared/ocr-language-packs.json");
    const MIB: u64 = 1024 * 1024;

    /// A language offered at launch: English is bundled, every other one is a pack with a budget.
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct LaunchLanguage {
        code: String,
        name: String,
        #[serde(default)]
        bundled: bool,
        pack_id: Option<String>,
        download_budget_bytes: Option<u64>,
        model: ModelPin,
        fixture: FixtureResult,
    }

    /// The language's model file in the Model baseline, pinned to the upstream commit.
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ModelPin {
        file: String,
        bytes: u64,
        git_blob_sha1: String,
        sha256: Option<String>,
    }

    /// The generated fixture page's text and what recognition must read from it.
    #[derive(Deserialize)]
    struct FixtureResult {
        text: String,
        recognizes: String,
    }

    fn launch_languages() -> Vec<LaunchLanguage> {
        let file: serde_json::Value = serde_json::from_str(LAUNCH_OCR_LANGUAGES).unwrap();
        serde_json::from_value(file["languages"].clone()).unwrap()
    }

    /// The release check on a catalog's OCR language packs: each is a launch language with a Model
    /// baseline entry, under its pack id, and no payload is over the language's download budget.
    fn check_ocr_language_packs(catalog: &Catalog, launch: &[LaunchLanguage]) -> Result<(), String> {
        for pack in &catalog.packs {
            let Some(language) = &pack.ocr_language else { continue };
            let baseline = launch.iter().find(|entry| entry.code == language.code && !entry.bundled)
                .ok_or_else(|| format!("{} reads {:?}, which has no Model baseline entry", pack.id, language.code))?;
            if baseline.pack_id.as_deref() != Some(pack.id.as_str()) {
                return Err(format!("{} reads {:?}, which belongs to pack {:?}", pack.id, language.code, baseline.pack_id));
            }
            let budget = baseline.download_budget_bytes.ok_or_else(|| format!("{} has no download budget", pack.id))?;
            if let Some(payload) = pack.payloads.iter().find(|payload| payload.download_bytes > budget) {
                return Err(format!("{} for {} is {} bytes, over its {budget} byte budget", pack.id, payload.target, payload.download_bytes));
            }
        }
        Ok(())
    }

    fn ocr_pack(id: &str, code: &str, download_bytes: u64) -> serde_json::Value {
        serde_json::json!({
            "id": id, "name": id, "description": "d", "toolIds": [OCR_TOOL_ID], "license": "Apache-2.0", "version": "1.0",
            "minPackFormat": 1, "appVersions": { "from": "0.1" }, "ocrLanguage": { "code": code, "name": code },
            "payloads": [{ "target": "all", "downloadBytes": download_bytes, "installedBytes": download_bytes, "sha256": "0".repeat(64), "path": format!("{id}.toolboxpack") }]
        })
    }

    fn catalog_of(packs: Vec<serde_json::Value>) -> Catalog {
        parse_catalog(&serde_json::json!({ "formatVersion": 1, "packs": packs }).to_string()).unwrap()
    }

    #[test]
    fn launch_ocr_languages_have_a_pinned_model_a_fixture_result_and_a_budget() {
        let launch = launch_languages();
        let hex = |text: &str, length: usize| text.len() == length && text.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        let mut codes = std::collections::BTreeSet::new();
        let mut pack_ids = std::collections::BTreeSet::new();
        for language in &launch {
            assert!(codes.insert(language.code.as_str()), "{} is listed once", language.code);
            assert!(!language.name.trim().is_empty());
            assert_eq!(language.model.file, format!("{}.traineddata", language.code));
            assert!(language.model.bytes > 0 && hex(&language.model.git_blob_sha1, 40), "{} pins its model", language.code);
            assert!(!language.fixture.recognizes.trim().is_empty() && language.fixture.text.contains(&language.fixture.recognizes), "{} has a fixture result", language.code);
            if language.bundled {
                assert_eq!((language.code.as_str(), language.name.as_str()), BUILT_IN_OCR_LANGUAGE);
                assert_eq!((&language.pack_id, language.download_budget_bytes), (&None, None));
                continue;
            }
            let pack_id = language.pack_id.as_deref().unwrap_or_else(|| panic!("{} names its pack", language.code));
            assert!(pack_ids.insert(pack_id));
            // Every language besides English checks a character only that language's model reads.
            assert!(!language.fixture.recognizes.is_ascii(), "{} fixture reads a script-specific character", language.code);
            // The model rounded up to a whole MiB, plus 1 MiB for the manifest, signature and archive.
            assert_eq!(language.download_budget_bytes, Some(language.model.bytes.div_ceil(MIB) * MIB + MIB), "{} budget", language.code);
        }
        assert_eq!(launch.iter().filter(|language| language.bundled).count(), 1);
        assert_eq!(launch.iter().filter(|language| !language.bundled).map(|language| language.code.as_str()).collect::<Vec<_>>(), ["deu", "fra", "spa", "hin", "guj", "tam"]);
        assert!(launch.iter().all(|language| language.download_budget_bytes.is_none_or(|budget| budget <= 5 * MIB)));
    }

    #[test]
    fn the_bundled_english_model_is_the_one_pinned_in_the_model_baseline() {
        use sha1::Digest as _;
        let english = launch_languages().into_iter().find(|language| language.bundled).unwrap();
        let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/resources/ocr/tessdata/eng.traineddata")).unwrap();
        assert_eq!(bytes.len() as u64, english.model.bytes);
        assert_eq!(english.model.sha256.as_deref(), Some(install::digest_hex(&bytes).as_str()));
        // The upstream repository names files by their Git blob hash, so the pin can be checked there.
        let mut blob = sha1::Sha1::new();
        blob.update(format!("blob {}\0", bytes.len()));
        blob.update(&bytes);
        let blob: String = blob.finalize().iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(blob, english.model.git_blob_sha1);
    }

    #[test]
    fn every_ocr_language_pack_in_the_built_in_catalog_is_a_launch_language_within_its_budget() {
        check_ocr_language_packs(&parse_catalog(BUILT_IN_CATALOG).unwrap(), &launch_languages()).unwrap();
    }

    #[test]
    fn the_release_check_refuses_an_ocr_pack_over_budget_or_missing_from_the_model_baseline() {
        let launch = launch_languages();
        let at_budget = launch.iter().filter(|language| !language.bundled)
            .map(|language| ocr_pack(language.pack_id.as_deref().unwrap(), &language.code, language.download_budget_bytes.unwrap()))
            .collect();
        check_ocr_language_packs(&catalog_of(at_budget), &launch).unwrap();

        let over = check_ocr_language_packs(&catalog_of(vec![ocr_pack("ocr-hindi", "hin", 3 * MIB + 1)]), &launch).unwrap_err();
        assert!(over.contains("over its 3145728 byte budget"), "{over}");
        let unpinned = check_ocr_language_packs(&catalog_of(vec![ocr_pack("ocr-japanese", "jpn", MIB)]), &launch).unwrap_err();
        assert!(unpinned.contains("no Model baseline entry"), "{unpinned}");
        let renamed = check_ocr_language_packs(&catalog_of(vec![ocr_pack("ocr-hindi-2", "hin", MIB)]), &launch).unwrap_err();
        assert!(renamed.contains("belongs to pack"), "{renamed}");
    }

    const SCRIPT_PACKS: &str = include_str!("../../shared/script-packs.json");

    /// The script pack plan: its download budget, the pack's id and name, and the families that
    /// move into packs of their own if the pack goes over budget.
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ScriptPackPlan {
        base_scripts: Vec<String>,
        download_budget_bytes: u64,
        pack: PlannedPack,
        families: Vec<ScriptFamily>,
    }

    #[derive(Deserialize)]
    struct PlannedPack {
        id: String,
        name: String,
    }

    #[derive(Deserialize)]
    struct ScriptFamily {
        id: String,
        name: String,
        scripts: Vec<String>,
    }

    fn script_plan() -> ScriptPackPlan {
        serde_json::from_str(SCRIPT_PACKS).unwrap()
    }

    /// The release check on a catalog's script packs: each is the planned pack or one of its
    /// families under the planned name, holds only scripts that belong to it, and no payload is
    /// over the budget.
    fn check_script_packs(catalog: &Catalog, plan: &ScriptPackPlan) -> Result<(), String> {
        let family_of = |script: &str| plan.families.iter().find(|family| family.scripts.iter().any(|name| name == script));
        for pack in catalog.packs.iter().filter(|pack| !pack.script_fonts.is_empty()) {
            let family = plan.families.iter().find(|family| family.id == pack.id);
            let name = match family {
                Some(family) => &family.name,
                None if pack.id == plan.pack.id => &plan.pack.name,
                None => return Err(format!("{} adds script fonts but isn't a planned script pack", pack.id)),
            };
            if &pack.name != name {
                return Err(format!("{} is named {:?}, not {name:?}", pack.id, pack.name));
            }
            for script in pack.script_fonts.keys() {
                let owner = family_of(script).map_or(plan.pack.id.as_str(), |family| family.id.as_str());
                // A family's scripts may stay in the main pack until it splits, never the reverse.
                if owner != pack.id && family.is_some() {
                    return Err(format!("{} adds {script}, which belongs to {owner}", pack.id));
                }
            }
            if let Some(payload) = pack.payloads.iter().find(|payload| payload.download_bytes > plan.download_budget_bytes) {
                return Err(format!("{} for {} is {} bytes, over the {} byte script pack budget", pack.id, payload.target, payload.download_bytes, plan.download_budget_bytes));
            }
        }
        Ok(())
    }

    fn script_pack(id: &str, name: &str, fonts: serde_json::Value, download_bytes: u64) -> serde_json::Value {
        serde_json::json!({
            "id": id, "name": name, "description": "d", "toolIds": [PDF_TEXT_TOOL_ID], "license": "OFL-1.1", "version": "1.0",
            "minPackFormat": 1, "appVersions": { "from": "0.1" }, "scriptFonts": fonts,
            "payloads": [{ "target": "all", "downloadBytes": download_bytes, "installedBytes": download_bytes, "sha256": "0".repeat(64), "path": format!("{id}.toolboxpack") }]
        })
    }

    #[test]
    fn the_script_pack_plan_has_a_budget_and_families_of_pack_scripts() {
        let plan = script_plan();
        assert_eq!(plan.download_budget_bytes, 40 * MIB);
        assert_eq!((plan.pack.id.as_str(), plan.pack.name.as_str()), ("unicode-scripts", "Unicode scripts"));
        // The app writes its base scripts itself, so no pack holds them.
        assert_eq!(plan.base_scripts, ["Latin", "Cyrillic", "Greek"]);
        let mut scripts = std::collections::BTreeSet::new();
        for family in &plan.families {
            assert!(family.id.starts_with("unicode-scripts-") && !family.name.is_empty());
            for script in &family.scripts {
                assert!(crate::kit::pdf::script_text::pack_script(script).is_some(), "{script}");
                assert!(scripts.insert(script.as_str()), "{script} is in two families");
            }
        }
        for script in &plan.base_scripts {
            assert!(crate::kit::pdf::script_text::pack_script(script).is_none(), "{script}");
        }
        check_script_packs(&parse_catalog(BUILT_IN_CATALOG).unwrap(), &plan).unwrap();
        check_script_packs(&fixture(), &plan).unwrap();
    }

    #[test]
    fn the_release_check_refuses_a_script_pack_over_budget_or_outside_the_plan() {
        let plan = script_plan();
        let fonts = serde_json::json!({ "Arabic": "fonts/Arabic.ttf", "Devanagari": "fonts/Devanagari.ttf", "Han": "fonts/Han.ttf" });
        check_script_packs(&catalog_of(vec![script_pack("unicode-scripts", "Unicode scripts", fonts.clone(), 40 * MIB)]), &plan).unwrap();
        // Split: the CJK family moves into its own pack.
        check_script_packs(&catalog_of(vec![
            script_pack("unicode-scripts", "Unicode scripts", serde_json::json!({ "Arabic": "fonts/Arabic.ttf" }), 20 * MIB),
            script_pack("unicode-scripts-cjk", "Unicode scripts: Chinese, Japanese and Korean", serde_json::json!({ "Han": "fonts/Han.ttf", "Hangul": "fonts/Hangul.ttf" }), 40 * MIB),
        ]), &plan).unwrap();

        let over = check_script_packs(&catalog_of(vec![script_pack("unicode-scripts", "Unicode scripts", fonts.clone(), 40 * MIB + 1)]), &plan).unwrap_err();
        assert!(over.contains("over the 41943040 byte script pack budget"), "{over}");
        let unplanned = check_script_packs(&catalog_of(vec![script_pack("unicode-fonts", "Unicode scripts", fonts.clone(), MIB)]), &plan).unwrap_err();
        assert!(unplanned.contains("isn't a planned script pack"), "{unplanned}");
        let renamed = check_script_packs(&catalog_of(vec![script_pack("unicode-scripts", "Fonts", fonts, MIB)]), &plan).unwrap_err();
        assert!(renamed.contains("not \"Unicode scripts\""), "{renamed}");
        let misplaced = check_script_packs(&catalog_of(vec![script_pack("unicode-scripts-cjk", "Unicode scripts: Chinese, Japanese and Korean", serde_json::json!({ "Arabic": "fonts/Arabic.ttf" }), MIB)]), &plan).unwrap_err();
        assert!(misplaced.contains("belongs to unicode-scripts"), "{misplaced}");
    }

    #[test]
    fn the_catalog_refuses_bad_script_fonts() {
        let parse = |pack: serde_json::Value| parse_catalog(&serde_json::json!({ "formatVersion": 1, "packs": [pack] }).to_string());
        let pack = |fonts: serde_json::Value| script_pack("unicode-scripts", "Unicode scripts", fonts, MIB);
        let catalog = parse(pack(serde_json::json!({ "Arabic": "fonts/Arabic.ttf" }))).unwrap();
        assert_eq!(catalog.packs[0].script_fonts["Arabic"], "fonts/Arabic.ttf");
        for (fonts, error) in [
            (serde_json::json!({ "Latin": "fonts/Latin.ttf" }), "bad script"),
            (serde_json::json!({ "Klingon": "fonts/Klingon.ttf" }), "bad script"),
            (serde_json::json!({ "Arabic": "../Arabic.ttf" }), "bad font"),
            (serde_json::json!({ "Arabic": "/fonts/Arabic.ttf" }), "bad font"),
            (serde_json::json!({ "Arabic": "fonts/Arabic.otf" }), "bad font"),
        ] {
            let message = parse(pack(fonts.clone())).unwrap_err();
            assert!(message.contains(error), "{fonts}: {message}");
        }
        let mut wrong_tool = pack(serde_json::json!({ "Arabic": "fonts/Arabic.ttf" }));
        wrong_tool["toolIds"] = serde_json::json!(["pdf-ocr"]);
        assert!(parse(wrong_tool).unwrap_err().contains("not to pdf-edit"));
        let twice = parse_catalog(&serde_json::json!({ "formatVersion": 1, "packs": [
            pack(serde_json::json!({ "Arabic": "fonts/Arabic.ttf" })),
            script_pack("unicode-scripts-cjk", "CJK", serde_json::json!({ "Arabic": "fonts/Arabic.ttf" }), MIB),
        ] }).to_string()).unwrap_err();
        assert!(twice.contains("more than one pack for script \"Arabic\""), "{twice}");
    }

    /// Listing must work offline: the module has no network code and reads only the filesystem.
    #[test]
    fn listing_makes_no_network_request() {
        let source = include_str!("packs.rs");
        let runtime = &source[..source.find("#[cfg(test)]\nmod tests").unwrap()];
        for needle in ["std::net", "TcpStream", "UdpSocket", "ToSocketAddrs", "reqwest", "ureq", "hyper", "tauri_plugin_http", "Command::new"] {
            assert!(!runtime.contains(needle), "packs.rs must not use {needle}");
        }
        // Listing a fixture works from the filesystem alone.
        let app_data = sandbox("offline");
        install(&app_data, "unicode-scripts", "1.0", record("1.0"));
        assert_eq!(list_packs_in(&fixture(), &app_data, MAC, APP).installed_count, 1);
    }
}
