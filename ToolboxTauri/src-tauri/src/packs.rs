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
}

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
