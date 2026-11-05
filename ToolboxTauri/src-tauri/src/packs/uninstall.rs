//! Uninstalling a pack, and the holds that runs keep on the packs they use. Local only.
//!
//! A run that resolves a resource from an installed pack holds that pack through its
//! [`PackHolds`], once per run however many resources it resolves. Holds are counted per pack
//! folder, and the run's holds are released when its `PackHolds` is dropped: when the run finishes
//! or is cancelled. Uninstall is refused with [`PackInstallErrorKind::InUse`] while any run holds
//! the pack.
//!
//! Uninstall moves `packs/<id>` out of `packs/` while the holds are locked, so a run can never
//! start using a pack that is half removed, and then deletes it before returning.

use super::install::{self, unavailable, PackInstallError, PackInstallErrorKind, INSTALL_LOCK};
use super::{list_packs_in, load_catalog, PackList, PACKS_DIR};
use crate::kit::resources::{resolve_pack_backed, PackBackedLocations, ResolvedResource, ResourceSource};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};

/// Private working folder in app data, beside `packs/` so the move out of `packs/` stays on one
/// volume. Anything left here by an interrupted uninstall is deleted by the next one.
const REMOVING_DIR: &str = "pack-removing";

/// How many runs hold each pack, by pack folder.
static HOLDS: LazyLock<Mutex<HashMap<PathBuf, usize>>> = LazyLock::new(Default::default);

fn holds() -> MutexGuard<'static, HashMap<PathBuf, usize>> {
    HOLDS.lock().unwrap_or_else(PoisonError::into_inner)
}

fn pack_folder(app_data: &Path, pack_id: &str) -> PathBuf {
    app_data.join(PACKS_DIR).join(pack_id)
}

/// The packs one run is using. Each pack is held once however often the run resolves it, and
/// every hold is released when this is dropped.
#[derive(Default)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct PackHolds {
    folders: BTreeSet<PathBuf>,
}

#[cfg_attr(not(test), allow(dead_code))]
impl PackHolds {
    /// Holds the pack for this run. Returns whether this run had not held it yet.
    pub fn hold(&mut self, app_data: &Path, pack_id: &str) -> bool {
        let folder = pack_folder(app_data, pack_id);
        if self.folders.contains(&folder) {
            return false;
        }
        *holds().entry(folder.clone()).or_default() += 1;
        self.folders.insert(folder);
        true
    }

    fn release(&mut self, folder: &Path) {
        if self.folders.remove(folder) {
            unhold(&mut holds(), folder);
        }
    }
}

impl Drop for PackHolds {
    fn drop(&mut self) {
        let mut holds = holds();
        for folder in &self.folders {
            unhold(&mut holds, folder);
        }
    }
}

fn unhold(holds: &mut HashMap<PathBuf, usize>, folder: &Path) {
    if let Some(count) = holds.get_mut(folder) {
        *count -= 1;
        if *count == 0 {
            holds.remove(folder);
        }
    }
}

/// How many runs hold the pack now.
#[cfg(test)]
fn holders(app_data: &Path, pack_id: &str) -> usize {
    holds().get(&pack_folder(app_data, pack_id)).copied().unwrap_or(0)
}

/// Resolves a pack-backed resource for a run. When it comes from the installed pack, the run holds
/// the pack until `holds` is dropped. The hold is taken before the pack is read, so an uninstall
/// either finishes first, and the resource is not resolved from the pack, or is refused.
#[cfg_attr(not(test), allow(dead_code))]
pub fn resolve_pack_resource(name: &str, app_data: &Path, pack_id: &str, bundled_root: Option<PathBuf>, development_override: Option<PathBuf>, holds: &mut PackHolds) -> Result<ResolvedResource, String> {
    let newly_held = holds.hold(app_data, pack_id);
    let locations = PackBackedLocations { bundled_root, installed_pack_root: super::installed_pack_root(app_data, pack_id), development_override };
    let resolved = resolve_pack_backed(name, &locations);
    let from_pack = matches!(&resolved, Ok(resource) if resource.source == ResourceSource::InstalledPack);
    if newly_held && !from_pack {
        holds.release(&pack_folder(app_data, pack_id));
    }
    resolved
}

/// Removes the pack and returns the refreshed pack list. Refused while a run holds the pack.
#[tauri::command]
pub async fn uninstall_pack(app: tauri::AppHandle, pack_id: String) -> Result<PackList, PackInstallError> {
    let app_data = super::app_data(&app).map_err(unavailable)?;
    let app_version = app.package_info().version.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let catalog = load_catalog().map_err(unavailable)?;
        if !catalog.packs.iter().any(|pack| pack.id == pack_id) {
            return Err(PackInstallError::new(PackInstallErrorKind::NotInCatalog, format!("The catalog has no pack {pack_id:?}")));
        }
        uninstall(&app_data, &pack_id)?;
        Ok(list_packs_in(&catalog, &app_data, &super::current_target(), &app_version))
    }).await.map_err(unavailable)?
}

/// Removes `packs/<pack_id>` and everything in it. Removing a pack that is not installed succeeds.
pub fn uninstall(app_data: &Path, pack_id: &str) -> Result<(), PackInstallError> {
    if !install::is_plain_segment(pack_id) {
        return Err(PackInstallError::new(PackInstallErrorKind::NotInCatalog, format!("{pack_id:?} is not a pack id")));
    }
    let _one_at_a_time = INSTALL_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let removing = app_data.join(REMOVING_DIR);
    install::remove_tree(&removing).map_err(unavailable)?;
    let folder = pack_folder(app_data, pack_id);
    {
        let holds = holds();
        if let Some(count) = holds.get(&folder) {
            return Err(PackInstallError::new(PackInstallErrorKind::InUse, format!("{count} run(s) are using {pack_id}")));
        }
        if std::fs::symlink_metadata(&folder).is_err() {
            return Ok(());
        }
        std::fs::create_dir_all(&removing).map_err(unavailable)?;
        std::fs::rename(&folder, removing.join(pack_id)).map_err(unavailable)?;
    }
    // The pack is no longer under packs/: discovery and listing no longer see it.
    install::remove_tree(&removing).map_err(unavailable)
}

#[cfg(test)]
mod tests {
    use super::super::install::tests::{sandbox, Fixture, TestKey};
    use super::super::install::{InstallContext, PackLimits};
    use super::super::{current_target, PackStatus};
    use super::*;

    const APP: &str = "1.0.14";
    const MODEL: &str = "tessdata/hin.traineddata";

    struct Setup {
        app_data: PathBuf,
        fixture: Fixture,
        catalog: super::super::Catalog,
    }

    impl Setup {
        /// Installs the fixture pack for real, so discovery can resolve its files.
        fn new(name: &str) -> Self {
            let root = sandbox(&format!("uninstall-{name}"));
            let key = TestKey::new(1);
            let fixture = Fixture::new();
            let file = root.join("pack.toolboxpack");
            std::fs::write(&file, fixture.file(&key)).unwrap();
            let catalog = fixture.catalog();
            let app_data = root.join("app-data");
            let target = current_target();
            install::install(&file, &InstallContext { catalog: &catalog, app_data: &app_data, target: &target, app_version: APP, public_key: &key.public_key(), limits: PackLimits::DEFAULT, cancel: None }).unwrap();
            Self { app_data, fixture, catalog }
        }

        fn list(&self) -> PackList {
            list_packs_in(&self.catalog, &self.app_data, &current_target(), APP)
        }

        fn resolve(&self, holds: &mut PackHolds) -> Result<ResolvedResource, String> {
            resolve_pack_resource(MODEL, &self.app_data, &self.fixture.id, None, None, holds)
        }

        fn uninstall(&self) -> Result<(), PackInstallError> {
            uninstall(&self.app_data, &self.fixture.id)
        }

        fn folder(&self) -> PathBuf {
            pack_folder(&self.app_data, &self.fixture.id)
        }
    }

    #[test]
    fn uninstall_removes_the_pack_folder_lists_it_as_not_installed_and_frees_its_space() {
        let setup = Setup::new("removes");
        // A leftover folder no catalog entry names keeps the total above zero.
        let leftover = setup.app_data.join(PACKS_DIR).join("retired-pack");
        std::fs::create_dir_all(&leftover).unwrap();
        std::fs::write(leftover.join("file"), vec![0u8; 321]).unwrap();
        let before = setup.list();
        let pack_bytes = before.packs[0].disk_bytes;
        assert_eq!((before.packs[0].status, before.installed_count), (PackStatus::Installed, 1));
        assert!(pack_bytes > 0);
        assert_eq!(before.total_bytes, pack_bytes + 321);

        setup.uninstall().unwrap();
        assert!(!setup.folder().exists());
        assert!(!setup.app_data.join(REMOVING_DIR).exists(), "nothing is left to delete later");
        let after = setup.list();
        let entry = &after.packs[0];
        assert_eq!((entry.status, entry.disk_bytes, entry.installed_version.as_deref()), (PackStatus::NotInstalled, 0, None));
        assert_eq!((after.installed_count, after.total_bytes), (0, before.total_bytes - pack_bytes));
        assert!(leftover.join("file").exists(), "other packs are left alone");

        // Removing it again changes nothing.
        setup.uninstall().unwrap();
        assert_eq!(setup.list().total_bytes, 321);
    }

    #[test]
    fn uninstall_is_refused_while_a_run_holds_the_pack_and_succeeds_once_it_finishes() {
        let setup = Setup::new("in-use");
        let mut run = PackHolds::default();
        assert_eq!(setup.resolve(&mut run).unwrap().source, ResourceSource::InstalledPack);
        // Resolving again in the same run does not hold it twice.
        setup.resolve(&mut run).unwrap();
        assert_eq!(holders(&setup.app_data, &setup.fixture.id), 1);

        let refused = setup.uninstall().unwrap_err();
        assert_eq!(refused.kind, PackInstallErrorKind::InUse);
        assert_eq!(setup.list().packs[0].status, PackStatus::Installed, "nothing was removed");
        assert!(setup.folder().exists());

        drop(run);
        assert_eq!(holders(&setup.app_data, &setup.fixture.id), 0);
        setup.uninstall().unwrap();
        assert_eq!(setup.list().packs[0].status, PackStatus::NotInstalled);
    }

    #[test]
    fn two_overlapping_runs_must_both_finish_before_the_pack_can_be_removed() {
        let setup = Setup::new("overlapping");
        let mut first = PackHolds::default();
        let mut second = PackHolds::default();
        setup.resolve(&mut first).unwrap();
        setup.resolve(&mut second).unwrap();
        assert_eq!(holders(&setup.app_data, &setup.fixture.id), 2);

        drop(first);
        assert_eq!(setup.uninstall().unwrap_err().kind, PackInstallErrorKind::InUse);
        assert!(setup.folder().exists());

        drop(second);
        setup.uninstall().unwrap();
        assert!(!setup.folder().exists());
    }

    #[test]
    fn after_uninstall_discovery_no_longer_resolves_the_pack_resource_and_holds_nothing() {
        let setup = Setup::new("discovery");
        let mut run = PackHolds::default();
        let resolved = setup.resolve(&mut run).unwrap();
        assert!(resolved.path.starts_with(setup.folder()));
        drop(run);

        setup.uninstall().unwrap();
        assert_eq!(super::super::installed_pack_root(&setup.app_data, &setup.fixture.id), None);
        let mut later = PackHolds::default();
        assert_eq!(setup.resolve(&mut later), Err(format!("Resource {MODEL} is unavailable.")));
        // A run that did not resolve anything from the pack does not keep it held.
        assert_eq!(holders(&setup.app_data, &setup.fixture.id), 0);
    }

    #[test]
    fn a_run_that_resolves_a_bundled_copy_does_not_hold_the_pack() {
        let setup = Setup::new("bundled");
        let bundled = setup.app_data.parent().unwrap().join("bundled");
        let model = setup.folder().join(&setup.fixture.version);
        std::fs::create_dir_all(bundled.join("tessdata")).unwrap();
        std::fs::copy(model.join(MODEL), bundled.join(MODEL)).unwrap();
        std::fs::copy(model.join("manifest.json"), bundled.join("manifest.json")).unwrap();
        let mut run = PackHolds::default();
        let resolved = resolve_pack_resource(MODEL, &setup.app_data, &setup.fixture.id, Some(bundled), None, &mut run).unwrap();
        assert_eq!(resolved.source, ResourceSource::Bundled);
        assert_eq!(holders(&setup.app_data, &setup.fixture.id), 0);
        setup.uninstall().unwrap();
    }

    #[test]
    fn holds_are_per_pack_and_a_stale_removal_is_cleared() {
        let setup = Setup::new("per-pack");
        let mut run = PackHolds::default();
        assert!(run.hold(&setup.app_data, "another-pack"));
        setup.uninstall().unwrap();
        drop(run);

        // An uninstall interrupted after the move leaves its folder behind; the next one deletes it.
        let stale = setup.app_data.join(REMOVING_DIR).join("ocr-hindi");
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::write(stale.join("file"), b"left over").unwrap();
        setup.uninstall().unwrap();
        assert!(!setup.app_data.join(REMOVING_DIR).exists());
    }

    #[test]
    fn ids_that_are_not_one_plain_folder_name_are_refused() {
        let setup = Setup::new("bad-id");
        for id in ["..", "../app-data", "", "a/b"] {
            assert_eq!(uninstall(&setup.app_data, id).unwrap_err().kind, PackInstallErrorKind::NotInCatalog, "{id:?}");
        }
        assert_eq!(setup.list().installed_count, 1);
    }

    #[test]
    fn uninstalling_makes_no_network_request() {
        let source = include_str!("uninstall.rs");
        let runtime = &source[..source.find("#[cfg(test)]\nmod tests").unwrap()];
        for needle in ["std::net", "TcpStream", "reqwest", "tauri_plugin_http", "Command::new"] {
            assert!(!runtime.contains(needle), "uninstall.rs must not use {needle}");
        }
    }
}
