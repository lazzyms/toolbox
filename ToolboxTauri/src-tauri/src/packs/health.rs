//! Whether an installed pack still matches what was installed, checked at its first use in each
//! app session. Local only.
//!
//! The first time a run uses a pack, every file in it is checked against the installed record and
//! the signed manifest, and the result is kept for the rest of the session. Later uses read the
//! kept result and no files. A pack that fails the check is damaged: nothing is resolved from it,
//! and nothing stands in for it, until it is installed again (Repair) or removed. Installing or
//! removing a pack forgets its result, so its next use checks it again.

use super::install::{digest_hex, is_plain_segment, INSTALL_LOCK};
use super::{files_intact, read_record, INSTALLED_FILE, MANIFEST_FILE, PACKS_DIR};
use crate::kit::resources::verify_manifest_root;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Health {
    /// Every file matched. The installed version's folder, which discovery reads.
    Verified(PathBuf),
    /// Why the pack cannot be used.
    Damaged(String),
}

/// The result of each pack's check this session, by pack folder.
static CHECKED: LazyLock<Mutex<HashMap<PathBuf, Health>>> = LazyLock::new(Default::default);

fn checked() -> MutexGuard<'static, HashMap<PathBuf, Health>> {
    CHECKED.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The pack's health, checked on its first use this session and kept after that. `None` when the
/// pack is not installed.
pub fn check(app_data: &Path, pack_id: &str) -> Option<Health> {
    let folder = app_data.join(PACKS_DIR).join(pack_id);
    if let Some(health) = checked().get(&folder) {
        return Some(health.clone());
    }
    // Not while an install or uninstall is replacing the files.
    let _one_at_a_time = INSTALL_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(health) = checked().get(&folder) {
        return Some(health.clone());
    }
    if std::fs::symlink_metadata(folder.join(INSTALLED_FILE)).is_err() {
        return None;
    }
    let health = verify(&folder).map_or_else(Health::Damaged, Health::Verified);
    checked().insert(folder, health.clone());
    Some(health)
}

/// Checks the pack again, after a file it was verified with failed when used.
pub fn recheck(app_data: &Path, pack_id: &str) -> Option<Health> {
    checked().remove(&app_data.join(PACKS_DIR).join(pack_id));
    check(app_data, pack_id)
}

/// Whether the pack in `folder` was found damaged this session. Reads no files.
pub fn known_damaged(folder: &Path) -> bool {
    matches!(checked().get(folder), Some(Health::Damaged(_)))
}

/// Forgets the result for the pack in `folder`, once its files were replaced or removed. Called
/// while the install lock is held.
pub(super) fn forget(folder: &Path) {
    checked().remove(folder);
}

fn verify(folder: &Path) -> Result<PathBuf, String> {
    let record = read_record(folder).ok_or("Its installed record cannot be read.")?;
    if !is_plain_segment(&record.version) {
        return Err("Its installed record names no version folder.".to_string());
    }
    let root = folder.join(&record.version);
    if !files_intact(&root, &record.files) {
        return Err("A file in it is missing or changed size.".to_string());
    }
    if let Some(expected) = &record.manifest_sha256 {
        let manifest = std::fs::read(root.join(MANIFEST_FILE)).map_err(|error| format!("Its manifest cannot be read: {error}"))?;
        if &digest_hex(&manifest) != expected {
            return Err("Its manifest was changed.".to_string());
        }
    }
    verify_manifest_root(&root)?;
    Ok(root)
}
