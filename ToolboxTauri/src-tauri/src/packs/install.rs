//! Installing a pack file, chosen by the person or downloaded by [`super::download`]. Local only:
//! nothing here uses the network.
//!
//! A pack file (`.toolboxpack`) is a zip envelope with three entries:
//! - `pack.json` — an unsigned header, `{ "formatVersion": 1, "id", "version", "target" }`. It says
//!   what the file claims to be so the person can confirm before anything is checked.
//! - `payload.zip` — the payload archive: `manifest.json` (the pack manifest) and the pack's files.
//! - `payload.zip.sig` — the minisign signature of `payload.zip`, base64 encoded like the
//!   updater's signatures and made with the updater key, so packs share one trust root with it.
//!
//! The pack manifest is a [`ResourceManifest`] plus a `pack` block that repeats the header and adds
//! `packFormat` and `appVersions`. It is trusted only after the payload's signature checks.
//!
//! Installing checks, in order: the signature, the archive checksum against the catalog, the
//! target and compatibility, then the archive size, unpacked size, file count and compression
//! limits while extracting into a private staging folder in app data, and then every extracted
//! file against the manifest with the shared verifier. Only then is the staged folder renamed into
//! `packs/<id>/<version>` and the installed record written. Any failure deletes staging and leaves
//! what was installed before as it was.

use super::{in_range, list_packs_in, load_catalog, Catalog, CatalogPack, InstalledRecord, PackList, Payload, ALL_TARGETS, INSTALLED_FILE, MANIFEST_FILE, PACKS_DIR, PACK_FORMAT};
use crate::kit::common::limits::CancelFlag;
use crate::kit::resources::{verify_resource, ResourceManifest, VerificationFailure};
use base64::Engine as _;
use minisign_verify::{PublicKey, Signature};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Cursor, Read, Seek};
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, PoisonError};

const ENVELOPE_FORMAT: u32 = 1;
const HEADER_ENTRY: &str = "pack.json";
const PAYLOAD_ENTRY: &str = "payload.zip";
const SIGNATURE_ENTRY: &str = "payload.zip.sig";
/// Private working folder in app data, beside `packs/` so the final rename stays on one volume.
const STAGING_DIR: &str = "pack-staging";
const BUILT_IN_CONFIG: &str = include_str!("../../tauri.conf.json");
/// Development override: the base64 minisign public key to trust instead of the updater key.
/// Release builds ignore it.
#[cfg(debug_assertions)]
const PUBLIC_KEY_OVERRIDE_ENV: &str = "TOOLBOX_PACK_PUBLIC_KEY";

/// One install or uninstall at a time, so two installs never share the staging folder and an
/// uninstall never removes a pack while it is being moved into place.
pub(super) static INSTALL_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackLimits {
    /// The payload archive inside the pack file.
    pub archive_bytes: u64,
    /// All extracted files together, counted as they are written.
    pub unpacked_bytes: u64,
    /// Entries in the payload archive, folders included.
    pub entries: usize,
    /// Extracted bytes per compressed byte of one entry.
    pub compression_ratio: u64,
    /// Entries that unpack to no more than this are exempt from the ratio, so tiny or very
    /// regular files are never rejected.
    pub ratio_exempt_bytes: u64,
    /// The header, the signature and the pack manifest, each.
    pub metadata_bytes: u64,
}

impl PackLimits {
    pub const DEFAULT: Self = Self {
        archive_bytes: 512 * 1024 * 1024,
        unpacked_bytes: 2 * 1024 * 1024 * 1024,
        entries: 10_000,
        compression_ratio: 100,
        ratio_exempt_bytes: 1024 * 1024,
        metadata_bytes: 1024 * 1024,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PackLimit {
    ArchiveSize,
    UnpackedSize,
    EntryCount,
    CompressionRatio,
}

/// Why a pack file was not installed. Each kind maps to its own plain-language copy in the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "details", rename_all = "camelCase")]
pub enum PackInstallErrorKind {
    /// Not a readable pack file: not a zip, or a missing or unreadable header or payload.
    NotAPack,
    /// The catalog built into this app has no pack with this id, version and target.
    NotInCatalog,
    /// No signature, or one that does not match the payload.
    BadSignature,
    /// Signed, but not with the key built into this app.
    WrongKey,
    /// The signed payload is not the archive the catalog lists.
    ChecksumMismatch,
    /// The signed payload is not laid out as a pack: no or invalid manifest, or a manifest that
    /// disagrees with the header.
    Malformed,
    /// Made for another Supported target, named here.
    #[serde(rename_all = "camelCase")]
    WrongTarget { target: String },
    /// Made in a pack format newer than this app reads.
    IncompatibleFormat,
    /// Made for app versions that do not include this one.
    IncompatibleApp,
    OverLimit { limit: PackLimit },
    /// An archive entry that is a link or would land outside the pack folder.
    UnsafeEntry { path: String },
    FileMissing { path: String },
    FileChecksumMismatch { path: String },
    FileSizeMismatch { path: String },
    /// A file the manifest does not list.
    UnlistedFile { path: String },
    /// Toolbox could not read its own settings or write to its app-data folder.
    Unavailable,
    /// The person cancelled it. Nothing was installed.
    Cancelled,
    /// The disk holding app data has less free space than the download and the installed pack
    /// need together. Checked before anything is downloaded.
    #[serde(rename_all = "camelCase")]
    NotEnoughSpace { needed_bytes: u64, free_bytes: u64 },
    /// No network: the release server's name could not be looked up, or there is no route.
    Offline,
    /// The network works but the release server could not be used: refused, timed out, a
    /// certificate or HTTP error, or a redirect away from its download hosts.
    ServerBlocked,
    /// The download started and then stopped or stalled before it finished.
    Interrupted,
    /// Uninstall only: a run is using the pack. Nothing was removed.
    InUse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackInstallError {
    #[serde(flatten)]
    pub kind: PackInstallErrorKind,
    /// Detail for logs. The UI shows copy chosen by `kind`.
    pub message: String,
}

impl PackInstallError {
    pub(super) fn new(kind: PackInstallErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }
}

type Result<T> = std::result::Result<T, PackInstallError>;

fn fail<T>(kind: PackInstallErrorKind, message: impl Into<String>) -> Result<T> {
    Err(PackInstallError::new(kind, message))
}

pub(super) fn unavailable(error: impl std::fmt::Display) -> PackInstallError {
    PackInstallError::new(PackInstallErrorKind::Unavailable, error.to_string())
}

/// What a pack file says it is, matched to the catalog. Shown before installing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackFileSummary {
    pub file_name: String,
    pub pack_id: String,
    pub name: String,
    pub version: String,
    pub installed_bytes: u64,
    pub license: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Header {
    format_version: u32,
    id: String,
    version: String,
    target: String,
}

#[derive(Deserialize)]
struct PackManifest {
    #[serde(flatten)]
    resources: ResourceManifest,
    pack: PackBlock,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackBlock {
    id: String,
    version: String,
    target: String,
    pack_format: u32,
    app_versions: super::VersionRange,
}

pub struct InstallContext<'a> {
    pub catalog: &'a Catalog,
    pub app_data: &'a Path,
    pub target: &'a str,
    pub app_version: &'a str,
    pub public_key: &'a PublicKey,
    pub limits: PackLimits,
    /// Checked between steps. Once the verified folder starts moving into place it is too late.
    pub cancel: Option<&'a CancelFlag>,
}

impl InstallContext<'_> {
    fn checkpoint(&self) -> Result<()> {
        match self.cancel {
            Some(cancel) if cancel.is_cancelled() => fail(PackInstallErrorKind::Cancelled, "The install was cancelled"),
            _ => Ok(()),
        }
    }
}

#[tauri::command]
pub async fn inspect_pack_file(path: PathBuf) -> Result<PackFileSummary> {
    tauri::async_runtime::spawn_blocking(move || {
        let catalog = load_catalog().map_err(unavailable)?;
        inspect(&path, &catalog, &super::current_target(), &PackLimits::DEFAULT)
    }).await.map_err(unavailable)?
}

/// Installs the chosen pack file and returns the refreshed pack list.
#[tauri::command]
pub async fn install_pack_file(app: tauri::AppHandle, path: PathBuf) -> Result<PackList> {
    let app_data = super::app_data(&app).map_err(unavailable)?;
    let app_version = app.package_info().version.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let catalog = load_catalog().map_err(unavailable)?;
        let public_key = trusted_public_key().map_err(unavailable)?;
        let target = super::current_target();
        let context = InstallContext { catalog: &catalog, app_data: &app_data, target: &target, app_version: &app_version, public_key: &public_key, limits: PackLimits::DEFAULT, cancel: None };
        install(&path, &context)?;
        Ok(list_packs_in(&catalog, &app_data, &target, &app_version))
    }).await.map_err(unavailable)?
}

/// The key pack payloads must be signed with: the updater public key built into this app.
pub fn trusted_public_key() -> std::result::Result<PublicKey, String> {
    #[cfg(debug_assertions)]
    if let Ok(encoded) = std::env::var(PUBLIC_KEY_OVERRIDE_ENV) {
        return decode_public_key(&encoded);
    }
    let config: serde_json::Value = serde_json::from_str(BUILT_IN_CONFIG).map_err(|error| error.to_string())?;
    let encoded = config["plugins"]["updater"]["pubkey"].as_str().ok_or("The app has no updater public key")?;
    decode_public_key(encoded)
}

/// Base64 of the minisign public key file, the format the updater configuration uses.
fn decode_public_key(encoded: &str) -> std::result::Result<PublicKey, String> {
    let text = base64::engine::general_purpose::STANDARD.decode(encoded.trim()).map_err(|error| error.to_string())?;
    PublicKey::decode(&String::from_utf8(text).map_err(|error| error.to_string())?).map_err(|error| error.to_string())
}

/// Reads only the header and matches it to the catalog. Nothing in the file is trusted yet.
pub fn inspect(path: &Path, catalog: &Catalog, target: &str, limits: &PackLimits) -> Result<PackFileSummary> {
    let mut envelope = open_envelope(path)?;
    let header = read_header(&mut envelope, limits)?;
    check_target(&header.target, target)?;
    let (pack, payload) = catalog_entry(catalog, &header)?;
    Ok(PackFileSummary {
        file_name: path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default(),
        pack_id: pack.id.clone(),
        name: pack.name.clone(),
        version: pack.version.clone(),
        installed_bytes: payload.installed_bytes,
        license: pack.license.clone(),
    })
}

/// Verifies the pack file completely and installs it, or changes nothing.
pub fn install(path: &Path, context: &InstallContext) -> Result<()> {
    let _one_at_a_time = INSTALL_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let staging = context.app_data.join(STAGING_DIR);
    // A crash during an earlier install can leave staging behind.
    remove_tree(&staging).map_err(unavailable)?;
    let result = install_staged(path, context, &staging);
    let cleaned = remove_tree(&staging);
    result?;
    cleaned.map_err(unavailable)
}

fn install_staged(path: &Path, context: &InstallContext, staging: &Path) -> Result<()> {
    let limits = &context.limits;
    context.checkpoint()?;
    let mut envelope = open_envelope(path)?;
    let header = read_header(&mut envelope, limits)?;
    let (_, catalog_payload) = catalog_entry(context.catalog, &header)?;
    let signature = read_entry(&mut envelope, SIGNATURE_ENTRY, limits.metadata_bytes)
        .map_err(|_| PackInstallError::new(PackInstallErrorKind::BadSignature, "The pack file has no readable signature"))?;
    let payload = read_payload(&mut envelope, limits.archive_bytes)?;
    context.checkpoint()?;
    verify_signature(&payload, &signature, context.public_key)?;
    context.checkpoint()?;
    if digest_hex(&payload) != catalog_payload.sha256 {
        return fail(PackInstallErrorKind::ChecksumMismatch, "The payload archive's checksum does not match the catalog");
    }
    context.checkpoint()?;

    let mut archive = zip::ZipArchive::new(Cursor::new(payload.as_slice()))
        .map_err(|error| PackInstallError::new(PackInstallErrorKind::Malformed, format!("The payload archive cannot be read: {error}")))?;
    let manifest = read_manifest(&mut archive, limits)?;
    let pack = &manifest.pack;
    if (pack.id.as_str(), pack.version.as_str(), pack.target.as_str()) != (header.id.as_str(), header.version.as_str(), header.target.as_str()) {
        return fail(PackInstallErrorKind::Malformed, "The signed manifest names another pack than the header");
    }
    check_target(&pack.target, context.target)?;
    if pack.pack_format == 0 || pack.pack_format > PACK_FORMAT {
        return fail(PackInstallErrorKind::IncompatibleFormat, format!("Pack format {} is not supported", pack.pack_format));
    }
    if !in_range(context.app_version, &pack.app_versions) {
        return fail(PackInstallErrorKind::IncompatibleApp, format!("The pack does not support app version {}", context.app_version));
    }
    let architecture = if pack.target == ALL_TARGETS { ALL_TARGETS } else { pack.target.split_once('-').map_or("", |(_, architecture)| architecture) };
    if manifest.resources.version != 1 || manifest.resources.architecture != architecture {
        return fail(PackInstallErrorKind::Malformed, "The manifest's version or architecture does not match its target");
    }

    let content = staging.join("content");
    fs::create_dir_all(&content).map_err(unavailable)?;
    extract(&mut archive, &content, context)?;
    let files = verify_files(&content, &manifest.resources, context)?;
    context.checkpoint()?;
    commit(context, &header, staging, &content, InstalledRecord {
        version: header.version.clone(),
        pack_format: pack.pack_format,
        app_versions: pack.app_versions.clone(),
        files,
    })
}

fn open_envelope(path: &Path) -> Result<zip::ZipArchive<fs::File>> {
    let not_a_pack = |error: &dyn std::fmt::Display| PackInstallError::new(PackInstallErrorKind::NotAPack, format!("The file is not a pack file: {error}"));
    let file = fs::File::open(path).map_err(|error| not_a_pack(&error))?;
    zip::ZipArchive::new(file).map_err(|error| not_a_pack(&error))
}

/// Reads a whole entry that must not exceed `limit` bytes.
fn read_entry<R: Read + Seek>(archive: &mut zip::ZipArchive<R>, name: &str, limit: u64) -> std::result::Result<Vec<u8>, String> {
    let entry = archive.by_name(name).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    entry.take(limit + 1).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > limit {
        return Err(format!("{name} is larger than {limit} bytes"));
    }
    Ok(bytes)
}

fn read_header<R: Read + Seek>(envelope: &mut zip::ZipArchive<R>, limits: &PackLimits) -> Result<Header> {
    let header: Header = read_entry(envelope, HEADER_ENTRY, limits.metadata_bytes)
        .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|error| error.to_string()))
        .map_err(|error| PackInstallError::new(PackInstallErrorKind::NotAPack, format!("The pack header cannot be read: {error}")))?;
    if header.format_version > ENVELOPE_FORMAT {
        return fail(PackInstallErrorKind::IncompatibleFormat, format!("Pack file format {} is not supported", header.format_version));
    }
    if header.format_version != ENVELOPE_FORMAT || ![&header.id, &header.version, &header.target].iter().all(|field| is_plain_segment(field)) {
        return fail(PackInstallErrorKind::NotAPack, "The pack header is not valid");
    }
    Ok(header)
}

/// A single file-name segment safe to use as a folder name: no separators, no leading dot.
pub fn is_plain_segment(text: &str) -> bool {
    !text.is_empty() && !text.starts_with('.') && text.len() <= 64
        && text.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'+'))
}

fn check_target(pack_target: &str, target: &str) -> Result<()> {
    if pack_target == ALL_TARGETS || pack_target == target {
        return Ok(());
    }
    fail(PackInstallErrorKind::WrongTarget { target: pack_target.to_string() }, format!("The pack is for {pack_target}, not {target}"))
}

/// The catalog pack and its payload for exactly the header's id, version and target.
fn catalog_entry<'a>(catalog: &'a Catalog, header: &Header) -> Result<(&'a CatalogPack, &'a Payload)> {
    catalog.packs.iter()
        .find(|pack| pack.id == header.id && pack.version == header.version)
        .and_then(|pack| pack.payloads.iter().find(|payload| payload.target == header.target).map(|payload| (pack, payload)))
        .ok_or_else(|| PackInstallError::new(PackInstallErrorKind::NotInCatalog, format!("The catalog has no {} {} for {}", header.id, header.version, header.target)))
}

fn read_payload<R: Read + Seek>(envelope: &mut zip::ZipArchive<R>, limit: u64) -> Result<Vec<u8>> {
    let entry = envelope.by_name(PAYLOAD_ENTRY)
        .map_err(|error| PackInstallError::new(PackInstallErrorKind::NotAPack, format!("The pack file has no payload: {error}")))?;
    if entry.size() > limit {
        return fail(PackInstallErrorKind::OverLimit { limit: PackLimit::ArchiveSize }, "The payload archive is too large");
    }
    let mut payload = Vec::new();
    entry.take(limit + 1).read_to_end(&mut payload)
        .map_err(|error| PackInstallError::new(PackInstallErrorKind::NotAPack, format!("The payload cannot be read: {error}")))?;
    if payload.len() as u64 > limit {
        return fail(PackInstallErrorKind::OverLimit { limit: PackLimit::ArchiveSize }, "The payload archive is too large");
    }
    Ok(payload)
}

/// Checks a base64 minisign signature the way the updater does, legacy signatures included.
fn verify_signature(payload: &[u8], encoded: &[u8], public_key: &PublicKey) -> Result<()> {
    let bad = |message: &str| PackInstallError::new(PackInstallErrorKind::BadSignature, message);
    let text = base64::engine::general_purpose::STANDARD.decode(encoded.trim_ascii()).ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .ok_or_else(|| bad("The signature is not encoded correctly"))?;
    let signature = Signature::decode(&text).map_err(|_| bad("The signature cannot be read"))?;
    match public_key.verify(payload, &signature, true) {
        Ok(()) => Ok(()),
        Err(minisign_verify::Error::UnexpectedKeyId) => fail(PackInstallErrorKind::WrongKey, "The payload is signed with another key"),
        Err(error) => Err(bad(&format!("The signature does not match the payload: {error}"))),
    }
}

fn read_manifest<R: Read + Seek>(archive: &mut zip::ZipArchive<R>, limits: &PackLimits) -> Result<PackManifest> {
    read_entry(archive, MANIFEST_FILE, limits.metadata_bytes)
        .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|error| error.to_string()))
        .map_err(|error| PackInstallError::new(PackInstallErrorKind::Malformed, format!("The pack manifest cannot be read: {error}")))
}

/// Relative paths made only of plain names, so they stay inside the folder they are joined to.
fn is_inside(path: &Path) -> bool {
    path.components().next().is_some() && path.components().all(|component| matches!(component, Component::Normal(_)))
}

/// Extracts every entry into `content`, enforcing the limits on the bytes actually written.
fn extract<R: Read + Seek>(archive: &mut zip::ZipArchive<R>, content: &Path, context: &InstallContext) -> Result<()> {
    let limits = &context.limits;
    if archive.len() > limits.entries {
        return fail(PackInstallErrorKind::OverLimit { limit: PackLimit::EntryCount }, format!("The payload has {} entries", archive.len()));
    }
    let mut unpacked = 0u64;
    for index in 0..archive.len() {
        context.checkpoint()?;
        let entry = archive.by_index(index)
            .map_err(|error| PackInstallError::new(PackInstallErrorKind::Malformed, format!("An archive entry cannot be read: {error}")))?;
        let name = entry.name().to_string();
        let unsafe_entry = || PackInstallError::new(PackInstallErrorKind::UnsafeEntry { path: name.clone() }, format!("Unsafe archive entry {name:?}"));
        let relative = PathBuf::from(name.trim_end_matches('/'));
        if entry.is_symlink() || name.contains('\\') || !is_inside(&relative) {
            return Err(unsafe_entry());
        }
        let destination = content.join(&relative);
        if entry.is_dir() {
            fs::create_dir_all(&destination).map_err(unavailable)?;
            continue;
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(unavailable)?;
        }
        // A repeated name would overwrite an earlier file, so it is refused.
        let mut file = match fs::OpenOptions::new().write(true).create_new(true).open(&destination) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Err(unsafe_entry()),
            Err(error) => return Err(unavailable(error)),
        };
        let remaining = limits.unpacked_bytes - unpacked;
        let ratio_cap = entry.compressed_size().saturating_mul(limits.compression_ratio).max(limits.ratio_exempt_bytes);
        let cap = remaining.min(ratio_cap);
        let written = std::io::copy(&mut entry.take(cap + 1), &mut file).map_err(|error| match error.kind() {
            std::io::ErrorKind::InvalidData | std::io::ErrorKind::UnexpectedEof => PackInstallError::new(PackInstallErrorKind::Malformed, format!("{name} cannot be unpacked: {error}")),
            _ => unavailable(error),
        })?;
        if written > cap {
            let limit = if remaining <= ratio_cap { PackLimit::UnpackedSize } else { PackLimit::CompressionRatio };
            return fail(PackInstallErrorKind::OverLimit { limit }, format!("{name} unpacks past the {limit:?} limit"));
        }
        unpacked += written;
    }
    Ok(())
}

/// Verifies every listed file with the shared verifier, refuses files the manifest does not list,
/// and returns every file's exact size for the installed record.
fn verify_files(content: &Path, manifest: &ResourceManifest, context: &InstallContext) -> Result<BTreeMap<String, u64>> {
    let mut listed = BTreeSet::from([PathBuf::from(MANIFEST_FILE)]);
    for spec in manifest.resources.values() {
        context.checkpoint()?;
        let path = spec.path.to_string_lossy().into_owned();
        if !is_inside(&spec.path) {
            return fail(PackInstallErrorKind::Malformed, format!("The manifest lists a path outside the pack: {path:?}"));
        }
        let kind = match verify_resource(&content.join(&spec.path), spec) {
            Ok(()) => {
                listed.insert(spec.path.components().collect());
                continue;
            }
            Err(VerificationFailure::Missing(_)) => PackInstallErrorKind::FileMissing { path },
            Err(VerificationFailure::OverSizeLimit) => PackInstallErrorKind::FileSizeMismatch { path },
            Err(VerificationFailure::ChecksumMismatch) => PackInstallErrorKind::FileChecksumMismatch { path },
            Err(failure @ (VerificationFailure::MissingVersion | VerificationFailure::MissingLicense)) => {
                return fail(PackInstallErrorKind::Malformed, format!("The manifest entry for {path:?} is incomplete: {failure:?}"));
            }
        };
        return fail(kind, "A pack file does not match its manifest");
    }
    let mut files = BTreeMap::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        for entry in fs::read_dir(content.join(&relative)).map_err(unavailable)? {
            let entry = entry.map_err(unavailable)?;
            let child = relative.join(entry.file_name());
            let metadata = entry.metadata().map_err(unavailable)?;
            if metadata.is_dir() {
                pending.push(child);
                continue;
            }
            let path = child.components().map(|component| component.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
            if !listed.contains(&child) {
                return fail(PackInstallErrorKind::UnlistedFile { path }, "The pack has a file its manifest does not list");
            }
            files.insert(path, metadata.len());
        }
    }
    Ok(files)
}

/// Moves the verified folder into place and records it. A folder already at that version (a
/// damaged or repeated install) is set aside first and restored if anything fails.
fn commit(context: &InstallContext, header: &Header, staging: &Path, content: &Path, record: InstalledRecord) -> Result<()> {
    let pack_folder = context.app_data.join(PACKS_DIR).join(&header.id);
    let created = !pack_folder.exists();
    fs::create_dir_all(&pack_folder).map_err(unavailable)?;
    let previous: Option<InstalledRecord> = fs::read_to_string(pack_folder.join(INSTALLED_FILE)).ok()
        .and_then(|text| serde_json::from_str(&text).ok());
    let destination = pack_folder.join(&header.version);
    let displaced = staging.join("displaced");
    let had_folder = fs::symlink_metadata(&destination).is_ok();
    let undo = |error: std::io::Error| {
        let _ = remove_tree(&destination);
        if had_folder {
            let _ = fs::rename(&displaced, &destination);
        } else if created {
            let _ = fs::remove_dir(&pack_folder);
        }
        unavailable(error)
    };
    if had_folder {
        fs::rename(&destination, &displaced).map_err(unavailable)?;
    }
    fs::rename(content, &destination).map_err(undo)?;
    let record_text = serde_json::to_vec_pretty(&record).map_err(unavailable)?;
    let pending_record = pack_folder.join(format!("{INSTALLED_FILE}.new"));
    fs::write(&pending_record, record_text)
        .and_then(|()| fs::rename(&pending_record, pack_folder.join(INSTALLED_FILE)))
        .map_err(|error| {
            let _ = fs::remove_file(&pending_record);
            undo(error)
        })?;
    // The older version is no longer recorded; removing it only frees space.
    if let Some(previous) = previous.filter(|previous| previous.version != header.version && is_plain_segment(&previous.version)) {
        let _ = remove_tree(&pack_folder.join(previous.version));
    }
    Ok(())
}

pub(super) fn remove_tree(path: &Path) -> std::io::Result<()> {
    match fs::remove_dir_all(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

pub(super) fn digest_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::{current_target, PackStatus};
    use super::*;
    use crate::kit::resources::{resolve_pack_backed, PackBackedLocations, ResourceSource, ResolvedResource};
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use std::io::Write;

    const APP: &str = "1.0.14";
    const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

    pub(in crate::packs) fn sandbox(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("toolbox-pack-install-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    /// A throwaway minisign key made for the test. It signs in the legacy mode the updater accepts.
    pub(in crate::packs) struct TestKey {
        pair: Ed25519KeyPair,
        id: [u8; 8],
    }

    impl TestKey {
        pub(in crate::packs) fn new(id: u8) -> Self {
            let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
            Self { pair: Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap(), id: [id; 8] }
        }

        /// Base64 of the minisign public key file, the updater configuration's format.
        fn encoded_public_key(&self) -> String {
            let key = [b"Ed".as_slice(), &self.id, self.pair.public_key().as_ref()].concat();
            B64.encode(format!("untrusted comment: minisign public key: test\n{}\n", B64.encode(key)))
        }

        pub(in crate::packs) fn public_key(&self) -> PublicKey {
            decode_public_key(&self.encoded_public_key()).unwrap()
        }

        /// Base64 of the minisign signature file, the updater's `.sig` format.
        fn sign(&self, data: &[u8]) -> String {
            let signature = self.pair.sign(data);
            let trusted = "timestamp:1700000000\tfile:payload.zip";
            let global = self.pair.sign(&[signature.as_ref(), trusted.as_bytes()].concat());
            let line = [b"Ed".as_slice(), &self.id, signature.as_ref()].concat();
            B64.encode(format!("untrusted comment: signature from test key\n{}\ntrusted comment: {trusted}\n{}\n", B64.encode(line), B64.encode(global.as_ref())))
        }
    }

    fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    /// A pack built in code. Each test changes what it needs and builds the file and its catalog.
    #[derive(Clone)]
    pub(in crate::packs) struct Fixture {
        pub(in crate::packs) id: String,
        pub(in crate::packs) version: String,
        pub(in crate::packs) target: String,
        pack_format: u32,
        app_versions: serde_json::Value,
        files: Vec<(String, Vec<u8>)>,
        /// Manifest entries per file: (path, sha256, maxBytes). Defaults to the real files.
        specs: Option<Vec<(String, String, u64)>>,
        extra_entries: Vec<(String, Vec<u8>)>,
    }

    impl Fixture {
        pub(in crate::packs) fn new() -> Self {
            Self {
                id: "ocr-hindi".into(),
                version: "2.0".into(),
                target: current_target(),
                pack_format: 1,
                app_versions: serde_json::json!({ "from": "1.0.0" }),
                files: vec![("tessdata/hin.traineddata".into(), b"hindi model".repeat(100)), ("LICENSE.txt".into(), b"Apache-2.0".to_vec())],
                specs: None,
                extra_entries: Vec::new(),
            }
        }

        fn architecture(&self) -> &str {
            if self.target == ALL_TARGETS { ALL_TARGETS } else { self.target.split_once('-').unwrap().1 }
        }

        fn manifest(&self) -> Vec<u8> {
            let specs = self.specs.clone().unwrap_or_else(|| self.files.iter().map(|(path, bytes)| (path.clone(), digest_hex(bytes), bytes.len() as u64)).collect());
            let resources: serde_json::Map<String, serde_json::Value> = specs.into_iter().map(|(path, sha256, max_bytes)| (path.clone(), serde_json::json!({
                "path": path, "version": self.version, "sha256": sha256, "license": "Apache-2.0", "maxBytes": max_bytes
            }))).collect();
            serde_json::to_vec(&serde_json::json!({
                "version": 1, "architecture": self.architecture(), "resources": resources,
                "pack": { "id": self.id, "version": self.version, "target": self.target, "packFormat": self.pack_format, "appVersions": self.app_versions }
            })).unwrap()
        }

        pub(in crate::packs) fn payload(&self) -> Vec<u8> {
            let manifest = self.manifest();
            let mut entries: Vec<(&str, &[u8])> = vec![(MANIFEST_FILE, &manifest)];
            entries.extend(self.files.iter().chain(&self.extra_entries).map(|(name, bytes)| (name.as_str(), bytes.as_slice())));
            zip(&entries)
        }

        fn header(&self) -> Vec<u8> {
            serde_json::to_vec(&serde_json::json!({ "formatVersion": 1, "id": self.id, "version": self.version, "target": self.target })).unwrap()
        }

        /// The pack file, its payload signed by `key`.
        pub(in crate::packs) fn file(&self, key: &TestKey) -> Vec<u8> {
            let payload = self.payload();
            Self::envelope(&self.header(), &payload, Some(key.sign(&payload).as_bytes()))
        }

        fn envelope(header: &[u8], payload: &[u8], signature: Option<&[u8]>) -> Vec<u8> {
            let mut entries = vec![(HEADER_ENTRY, header), (PAYLOAD_ENTRY, payload)];
            entries.extend(signature.map(|signature| (SIGNATURE_ENTRY, signature)));
            zip(&entries)
        }

        /// A catalog offering exactly this pack, with the payload's real checksum.
        pub(in crate::packs) fn catalog(&self) -> Catalog {
            self.catalog_with_checksum(digest_hex(&self.payload()))
        }

        fn catalog_with_checksum(&self, sha256: String) -> Catalog {
            super::super::parse_catalog(&serde_json::json!({ "formatVersion": 1, "packs": [{
                "id": self.id, "name": "Text recognition, Hindi", "description": "Reads Hindi.", "toolIds": ["pdf-ocr"],
                "license": "Apache-2.0", "version": self.version, "minPackFormat": 1, "appVersions": { "from": "1.0.0" },
                "payloads": [{ "target": self.target, "downloadBytes": 1000, "installedBytes": 32505856, "sha256": sha256, "path": "packs/ocr-hindi.toolboxpack" }]
            }]}).to_string()).unwrap()
        }

        fn installed_bytes(&self) -> u64 {
            self.files.iter().map(|(_, bytes)| bytes.len() as u64).sum::<u64>() + self.manifest().len() as u64
        }
    }

    struct Setup {
        app_data: PathBuf,
        file: PathBuf,
        catalog: Catalog,
        key: PublicKey,
        limits: PackLimits,
    }

    impl Setup {
        fn new(name: &str, fixture: &Fixture, key: &TestKey) -> Self {
            let root = sandbox(name);
            let file = root.join(format!("{}-{}.toolboxpack", fixture.id, fixture.version));
            fs::write(&file, fixture.file(key)).unwrap();
            Self { app_data: root.join("app-data"), file, catalog: fixture.catalog(), key: key.public_key(), limits: PackLimits::DEFAULT }
        }

        fn install(&self) -> Result<()> {
            self.install_as(APP)
        }

        fn install_as(&self, app_version: &str) -> Result<()> {
            let target = current_target();
            install(&self.file, &InstallContext { catalog: &self.catalog, app_data: &self.app_data, target: &target, app_version, public_key: &self.key, limits: self.limits, cancel: None })
        }

        fn list(&self, app_version: &str) -> PackList {
            list_packs_in(&self.catalog, &self.app_data, &current_target(), app_version)
        }

        fn locations(&self, bundled_root: &Path) -> PackBackedLocations {
            PackBackedLocations { bundled_root: Some(bundled_root.to_path_buf()), installed_pack_root: super::super::installed_pack_root(&self.app_data, "ocr-hindi"), development_override: None }
        }

        /// Nothing visible under packs/ for this pack and nothing left in staging.
        fn assert_untouched(&self, case: &str) {
            assert!(!self.app_data.join(PACKS_DIR).join("ocr-hindi").exists(), "{case}: a pack folder was left");
            assert!(!self.app_data.join(STAGING_DIR).exists(), "{case}: staging was left");
            assert_eq!(self.list(APP).installed_count, 0, "{case}");
        }
    }

    fn kind_of(result: Result<()>) -> PackInstallErrorKind {
        result.expect_err("the pack should be rejected").kind
    }

    #[test]
    fn a_valid_pack_signed_by_the_development_override_key_installs_lists_and_resolves() {
        let key = TestKey::new(1);
        let fixture = Fixture::new();
        let mut setup = Setup::new("valid", &fixture, &key);
        {
            let _guard = crate::kit::lock_process_env();
            std::env::set_var(PUBLIC_KEY_OVERRIDE_ENV, key.encoded_public_key());
            let overridden = trusted_public_key();
            std::env::remove_var(PUBLIC_KEY_OVERRIDE_ENV);
            setup.key = overridden.unwrap();
        }
        let original = fs::read(&setup.file).unwrap();
        setup.install().unwrap();

        let list = setup.list(APP);
        let entry = &list.packs[0];
        assert_eq!((entry.status, entry.installed_version.as_deref()), (PackStatus::Installed, Some("2.0")));
        let record_bytes = fs::metadata(setup.app_data.join("packs/ocr-hindi/installed.json")).unwrap().len();
        assert_eq!(entry.disk_bytes, fixture.installed_bytes() + record_bytes);
        assert_eq!((list.installed_count, list.total_bytes), (1, entry.disk_bytes));
        assert!(!setup.app_data.join(STAGING_DIR).exists());

        let model = setup.app_data.join("packs/ocr-hindi/2.0/tessdata/hin.traineddata");
        assert_eq!(resolve_pack_backed("tessdata/hin.traineddata", &setup.locations(&sandbox("valid-bundled"))), Ok(ResolvedResource { path: model, source: ResourceSource::InstalledPack }));
        // The chosen file is never changed.
        assert_eq!(fs::read(&setup.file).unwrap(), original);
    }

    #[test]
    fn installed_packs_survive_a_simulated_app_update() {
        let key = TestKey::new(1);
        let setup = Setup::new("survive", &Fixture::new(), &key);
        setup.install().unwrap();
        // The update replaces the app and its resource root; app data stays.
        let new_resources = sandbox("survive-new-resources");
        let list = setup.list("1.0.15");
        assert_eq!(list.packs[0].status, PackStatus::Installed);
        let resolved = resolve_pack_backed("LICENSE.txt", &setup.locations(&new_resources)).unwrap();
        assert_eq!(resolved.source, ResourceSource::InstalledPack);
        assert!(resolved.path.starts_with(&setup.app_data));
    }

    #[test]
    fn each_rejection_has_its_own_typed_error_and_leaves_no_pack_and_no_staging() {
        use PackInstallErrorKind as Kind;
        let key = TestKey::new(1);
        let other_target = if current_target() == "windows-x86_64" { "macos-aarch64" } else { "windows-x86_64" };
        let path = |text: &str| text.to_string();
        let good = Fixture::new();
        let model_sha = digest_hex(&good.files[0].1);
        let license_spec = (path("LICENSE.txt"), digest_hex(&good.files[1].1), good.files[1].1.len() as u64);

        type Case = (&'static str, Box<dyn Fn(&mut Setup)>, Kind);
        let rewrite = |build: Box<dyn Fn(&Fixture, &TestKey, &TestKey) -> (Vec<u8>, Option<Catalog>)>| -> Box<dyn Fn(&mut Setup)> {
            let (good, key, other_key) = (Fixture::new(), TestKey::new(1), TestKey::new(2));
            Box::new(move |setup: &mut Setup| {
                let (bytes, catalog) = build(&good, &key, &other_key);
                fs::write(&setup.file, bytes).unwrap();
                if let Some(catalog) = catalog { setup.catalog = catalog; }
                setup.key = key.public_key();
            })
        };
        let with = |change: Box<dyn Fn(&mut Fixture)>| rewrite(Box::new(move |good, key, _| {
            let mut fixture = good.clone();
            change(&mut fixture);
            (fixture.file(key), Some(fixture.catalog()))
        }));
        let limit = |change: fn(&mut PackLimits)| -> Box<dyn Fn(&mut Setup)> { Box::new(move |setup: &mut Setup| change(&mut setup.limits)) };

        let cases: Vec<Case> = vec![
            ("bad signature", rewrite(Box::new(|good, key, _| {
                (Fixture::envelope(&good.header(), &good.payload(), Some(key.sign(b"something else").as_bytes())), None)
            })), Kind::BadSignature),
            ("no signature", rewrite(Box::new(|good, _, _| (Fixture::envelope(&good.header(), &good.payload(), None), None))), Kind::BadSignature),
            ("wrong key", rewrite(Box::new(|good, _, other_key| (good.file(other_key), None))), Kind::WrongKey),
            ("archive checksum mismatch", rewrite(Box::new(|good, key, _| (good.file(key), Some(good.catalog_with_checksum("0".repeat(64)))))), Kind::ChecksumMismatch),
            ("manifest file checksum mismatch", with(Box::new(move |fixture| {
                fixture.specs = Some(vec![(path("tessdata/hin.traineddata"), digest_hex(b"other"), 1100), license_spec.clone()]);
            })), Kind::FileChecksumMismatch { path: path("tessdata/hin.traineddata") }),
            ("manifest file size mismatch", with(Box::new({
                let model_sha = model_sha.clone();
                move |fixture| fixture.specs = Some(vec![(path("tessdata/hin.traineddata"), model_sha.clone(), 1099), (path("LICENSE.txt"), digest_hex(b"Apache-2.0"), 10)])
            })), Kind::FileSizeMismatch { path: path("tessdata/hin.traineddata") }),
            ("listed file missing", with(Box::new(|fixture| {
                let specs = fixture.files.iter().map(|(path, bytes)| (path.clone(), digest_hex(bytes), bytes.len() as u64)).collect();
                fixture.specs = Some(specs);
                fixture.files.remove(1);
            })), Kind::FileMissing { path: path("LICENSE.txt") }),
            ("unlisted file", with(Box::new(|fixture| fixture.extra_entries.push(("extra.bin".into(), b"x".to_vec())))), Kind::UnlistedFile { path: path("extra.bin") }),
            ("wrong target", with(Box::new(move |fixture| fixture.target = other_target.into())), Kind::WrongTarget { target: path(other_target) }),
            ("incompatible pack format", with(Box::new(|fixture| fixture.pack_format = PACK_FORMAT + 1)), Kind::IncompatibleFormat),
            ("incompatible app range", with(Box::new(|fixture| fixture.app_versions = serde_json::json!({ "from": "0.1.0", "before": "1.0.0" }))), Kind::IncompatibleApp),
            ("archive over its size limit", limit(|limits| limits.archive_bytes = 256), Kind::OverLimit { limit: PackLimit::ArchiveSize }),
            ("over the unpacked size limit", limit(|limits| { limits.unpacked_bytes = 1050; }), Kind::OverLimit { limit: PackLimit::UnpackedSize }),
            ("over the entry count limit", limit(|limits| limits.entries = 2), Kind::OverLimit { limit: PackLimit::EntryCount }),
            ("over the decompression ratio limit", with(Box::new(|fixture| fixture.files.push(("zeros.bin".into(), vec![0; 4 * 1024 * 1024])))), Kind::OverLimit { limit: PackLimit::CompressionRatio }),
            ("entry escaping the pack folder", with(Box::new(|fixture| fixture.extra_entries.push(("../escape.bin".into(), b"x".to_vec())))), Kind::UnsafeEntry { path: path("../escape.bin") }),
            ("not a pack file", rewrite(Box::new(|_, _, _| (b"not a zip".to_vec(), None))), Kind::NotAPack),
            ("not in the catalog", rewrite(Box::new(|good, key, _| {
                let mut newer = good.clone();
                newer.version = "2.1".into();
                (newer.file(key), None)
            })), Kind::NotInCatalog),
            ("manifest naming another pack", rewrite(Box::new(|good, key, _| {
                let mut other = good.clone();
                other.version = "2.1".into();
                let payload = other.payload();
                let catalog = good.catalog_with_checksum(digest_hex(&payload));
                (Fixture::envelope(&good.header(), &payload, Some(key.sign(&payload).as_bytes())), Some(catalog))
            })), Kind::Malformed),
        ];
        for (name, arrange, expected) in cases {
            let mut setup = Setup::new(&format!("reject-{}", name.replace(' ', "-")), &good, &key);
            arrange(&mut setup);
            let kind = setup.install().map(|()| panic!("{name}: the pack should be rejected")).unwrap_err().kind;
            assert_eq!(kind, expected, "{name}");
            setup.assert_untouched(name);
        }
    }

    #[test]
    fn a_symbolic_link_in_the_payload_is_refused() {
        let key = TestKey::new(1);
        let fixture = Fixture::new();
        let manifest = fixture.manifest();
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file(MANIFEST_FILE, options).unwrap();
        writer.write_all(&manifest).unwrap();
        writer.add_symlink("link", "/etc/passwd", options).unwrap();
        let payload = writer.finish().unwrap().into_inner();
        let mut setup = Setup::new("symlink", &fixture, &key);
        fs::write(&setup.file, Fixture::envelope(&fixture.header(), &payload, Some(key.sign(&payload).as_bytes()))).unwrap();
        setup.catalog = fixture.catalog_with_checksum(digest_hex(&payload));
        assert_eq!(kind_of(setup.install()), PackInstallErrorKind::UnsafeEntry { path: "link".into() });
        setup.assert_untouched("symlink");
    }

    #[test]
    fn a_failed_install_keeps_the_previous_install_and_an_update_replaces_it() {
        let key = TestKey::new(1);
        let mut old = Fixture::new();
        old.version = "1.0".into();
        let setup = Setup::new("update", &old, &key);
        setup.install().unwrap();
        let old_record = fs::read(setup.app_data.join("packs/ocr-hindi/installed.json")).unwrap();

        // A tampered 2.0 is rejected and 1.0 stays exactly as it was.
        let new = Fixture::new();
        let mut tampered = new.file(&key);
        let at = tampered.len() / 2;
        tampered[at] ^= 0xff;
        fs::write(&setup.file, &tampered).unwrap();
        let catalog = new.catalog();
        let target = current_target();
        let context = InstallContext { catalog: &catalog, app_data: &setup.app_data, target: &target, app_version: APP, public_key: &setup.key, limits: PackLimits::DEFAULT, cancel: None };
        assert!(install(&setup.file, &context).is_err());
        assert_eq!(fs::read(setup.app_data.join("packs/ocr-hindi/installed.json")).unwrap(), old_record);
        assert!(setup.app_data.join("packs/ocr-hindi/1.0/tessdata/hin.traineddata").is_file());
        assert!(!setup.app_data.join(STAGING_DIR).exists());

        // The genuine 2.0 replaces it, and the 1.0 folder is removed.
        fs::write(&setup.file, new.file(&key)).unwrap();
        install(&setup.file, &context).unwrap();
        let list = list_packs_in(&catalog, &setup.app_data, &target, APP);
        assert_eq!((list.packs[0].status, list.packs[0].installed_version.as_deref()), (PackStatus::Installed, Some("2.0")));
        assert!(!setup.app_data.join("packs/ocr-hindi/1.0").exists());

        // A damaged install of the same version is repaired by installing the file again.
        fs::remove_file(setup.app_data.join("packs/ocr-hindi/2.0/LICENSE.txt")).unwrap();
        assert_eq!(list_packs_in(&catalog, &setup.app_data, &target, APP).packs[0].status, PackStatus::Damaged);
        install(&setup.file, &context).unwrap();
        assert_eq!(list_packs_in(&catalog, &setup.app_data, &target, APP).packs[0].status, PackStatus::Installed);
    }

    #[test]
    fn stale_staging_from_an_interrupted_install_is_cleared() {
        let key = TestKey::new(1);
        let setup = Setup::new("stale", &Fixture::new(), &key);
        fs::create_dir_all(setup.app_data.join(STAGING_DIR).join("content/leftover")).unwrap();
        setup.install().unwrap();
        assert!(!setup.app_data.join(STAGING_DIR).exists());
    }

    /// The limits are part of the pack contract; changing them must be deliberate.
    #[test]
    fn the_install_limits_are_recorded() {
        assert_eq!(PackLimits::DEFAULT, PackLimits {
            archive_bytes: 512 * 1024 * 1024,
            unpacked_bytes: 2 * 1024 * 1024 * 1024,
            entries: 10_000,
            compression_ratio: 100,
            ratio_exempt_bytes: 1024 * 1024,
            metadata_bytes: 1024 * 1024,
        });
    }

    #[test]
    fn inspecting_reads_the_header_and_catalog_and_reports_early_problems() {
        let key = TestKey::new(1);
        let fixture = Fixture::new();
        let setup = Setup::new("inspect", &fixture, &key);
        let target = current_target();
        let summary = inspect(&setup.file, &setup.catalog, &target, &PackLimits::DEFAULT).unwrap();
        assert_eq!(summary, PackFileSummary {
            file_name: "ocr-hindi-2.0.toolboxpack".into(), pack_id: "ocr-hindi".into(), name: "Text recognition, Hindi".into(),
            version: "2.0".into(), installed_bytes: 32505856, license: "Apache-2.0".into(),
        });
        assert!(!setup.app_data.exists(), "inspecting writes nothing");
        let other = if target == "windows-x86_64" { "macos-aarch64" } else { "windows-x86_64" };
        assert_eq!(inspect(&setup.file, &setup.catalog, other, &PackLimits::DEFAULT).unwrap_err().kind, PackInstallErrorKind::WrongTarget { target });
        let mut unknown = fixture.clone();
        unknown.id = "ocr-tamil".into();
        assert_eq!(inspect(&setup.file, &unknown.catalog(), &current_target(), &PackLimits::DEFAULT).unwrap_err().kind, PackInstallErrorKind::NotInCatalog);
        fs::write(&setup.file, b"plain text").unwrap();
        assert_eq!(inspect(&setup.file, &setup.catalog, &current_target(), &PackLimits::DEFAULT).unwrap_err().kind, PackInstallErrorKind::NotAPack);
        let mut header_only = fixture.clone();
        header_only.version = "../2.0".into();
        fs::write(&setup.file, Fixture::envelope(&header_only.header(), b"", None)).unwrap();
        assert_eq!(inspect(&setup.file, &setup.catalog, &current_target(), &PackLimits::DEFAULT).unwrap_err().kind, PackInstallErrorKind::NotAPack);
    }

    #[test]
    fn errors_serialize_with_a_kind_and_details_for_the_ui() {
        let error = PackInstallError::new(PackInstallErrorKind::WrongTarget { target: "windows-aarch64".into() }, "detail");
        assert_eq!(serde_json::to_value(&error).unwrap(), serde_json::json!({ "kind": "wrongTarget", "details": { "target": "windows-aarch64" }, "message": "detail" }));
        let error = PackInstallError::new(PackInstallErrorKind::OverLimit { limit: PackLimit::CompressionRatio }, "detail");
        assert_eq!(serde_json::to_value(&error).unwrap()["details"], serde_json::json!({ "limit": "compressionRatio" }));
        assert_eq!(serde_json::to_value(PackInstallError::new(PackInstallErrorKind::BadSignature, "")).unwrap()["kind"], "badSignature");
        let error = PackInstallError::new(PackInstallErrorKind::NotEnoughSpace { needed_bytes: 310, free_bytes: 120 }, "");
        assert_eq!(serde_json::to_value(&error).unwrap()["details"], serde_json::json!({ "neededBytes": 310, "freeBytes": 120 }));
        for (kind, name) in [(PackInstallErrorKind::Offline, "offline"), (PackInstallErrorKind::ServerBlocked, "serverBlocked"), (PackInstallErrorKind::Interrupted, "interrupted"), (PackInstallErrorKind::Cancelled, "cancelled"), (PackInstallErrorKind::InUse, "inUse")] {
            assert_eq!(serde_json::to_value(PackInstallError::new(kind, "")).unwrap()["kind"], name);
        }
    }

    #[test]
    fn a_cancelled_install_stops_before_anything_is_visible_and_a_later_install_works() {
        let key = TestKey::new(1);
        let setup = Setup::new("cancelled", &Fixture::new(), &key);
        let target = current_target();
        let cancel = CancelFlag::default();
        cancel.cancel();
        let context = InstallContext { catalog: &setup.catalog, app_data: &setup.app_data, target: &target, app_version: APP, public_key: &setup.key, limits: setup.limits, cancel: Some(&cancel) };
        assert_eq!(kind_of(install(&setup.file, &context)), PackInstallErrorKind::Cancelled);
        setup.assert_untouched("cancelled");
        setup.install().unwrap();
        assert_eq!(setup.list(APP).installed_count, 1);
    }

    #[test]
    fn the_updater_key_built_into_the_app_is_the_trust_root() {
        let _guard = crate::kit::lock_process_env();
        std::env::remove_var(PUBLIC_KEY_OVERRIDE_ENV);
        let built_in = trusted_public_key().unwrap();
        // A payload signed by any other key is refused as the wrong key.
        let key = TestKey::new(9);
        let payload = b"payload";
        assert_eq!(verify_signature(payload, key.sign(payload).as_bytes(), &built_in).unwrap_err().kind, PackInstallErrorKind::WrongKey);
    }

    /// The prehashed signatures `minisign` and the Tauri signer make are accepted too.
    #[test]
    fn a_prehashed_minisign_signature_verifies() {
        let public_key = PublicKey::from_base64("RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3").unwrap();
        let signature = "untrusted comment: signature from minisign secret key\nRUQf6LRCGA9i559r3g7V1qNyJDApGip8MfqcadIgT9CuhV3EMhHoN1mGTkUidF/z7SrlQgXdy8ofjb7bNJJylDOocrCo8KLzZwo=\ntrusted comment: timestamp:1556193335\tfile:test\ny/rUw2y8/hOUYjZU71eHp/Wo1KZ40fGy2VJEDl34XMJM+TX48Ss/17u3IvIfbVR1FkZZSNCisQbuQY+bHwhEBg==";
        let encoded = B64.encode(signature);
        assert_eq!(verify_signature(b"test", encoded.as_bytes(), &public_key), Ok(()));
        assert_eq!(verify_signature(b"tess", encoded.as_bytes(), &public_key).unwrap_err().kind, PackInstallErrorKind::BadSignature);
    }

    /// Installing from a file must work offline: this module has no network code.
    #[test]
    fn installing_makes_no_network_request() {
        let source = include_str!("install.rs");
        let runtime = &source[..source.find("#[cfg(test)]\npub(super) mod tests").unwrap()];
        for needle in ["std::net", "TcpStream", "UdpSocket", "ToSocketAddrs", "reqwest", "ureq", "hyper", "tauri_plugin_http", "Command::new"] {
            assert!(!runtime.contains(needle), "install.rs must not use {needle}");
        }
    }
}
