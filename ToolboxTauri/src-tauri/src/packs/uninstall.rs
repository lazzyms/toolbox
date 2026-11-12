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

use super::health::{self, Health};
use super::install::{self, unavailable, PackInstallError, PackInstallErrorKind, INSTALL_LOCK};
use super::{list_packs_in, load_catalog, PackList, PACKS_DIR};
use crate::kit::contracts::{OutcomePointer, ToolError};
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
///
/// The pack is checked at its first use this session ([`health`]). A damaged pack fails with
/// [`ErrorKind::Unavailable`](crate::kit::contracts::ErrorKind) pointing to the pack, and nothing
/// past it, such as a development override, is used in its place. Each file the run would process
/// with the resource fails with this error.
#[cfg_attr(not(test), allow(dead_code))]
pub fn resolve_pack_resource(name: &str, app_data: &Path, pack_id: &str, bundled_root: Option<PathBuf>, development_override: Option<PathBuf>, holds: &mut PackHolds) -> Result<ResolvedResource, ToolError> {
    let newly_held = holds.hold(app_data, pack_id);
    let resolved = resolve_checked(name, app_data, pack_id, bundled_root, development_override);
    let from_pack = matches!(&resolved, Ok(resource) if resource.source == ResourceSource::InstalledPack);
    if newly_held && !from_pack {
        holds.release(&pack_folder(app_data, pack_id));
    }
    resolved
}

fn resolve_checked(name: &str, app_data: &Path, pack_id: &str, bundled_root: Option<PathBuf>, development_override: Option<PathBuf>) -> Result<ResolvedResource, ToolError> {
    let damaged = |reason: &str| ToolError::unavailable(format!("The {pack_id} pack is damaged. {reason} Repair it in Settings, Packs."))
        .with_pointer(OutcomePointer::Pack { pack_id: pack_id.to_string() });
    let installed_pack_root = match health::check(app_data, pack_id) {
        None => None,
        Some(Health::Verified(root)) => Some(root),
        Some(Health::Damaged(reason)) => {
            // Only a bundled copy, which never comes from the pack, may still be used.
            let bundled = PackBackedLocations { bundled_root, installed_pack_root: None, development_override: None };
            return resolve_pack_backed(name, &bundled).map_err(|_| damaged(&reason));
        }
    };
    let verified = installed_pack_root.is_some();
    let locations = PackBackedLocations { bundled_root, installed_pack_root, development_override };
    resolve_pack_backed(name, &locations).map_err(|error| {
        // A file that passed the check this session no longer does: the pack is damaged now.
        match verified.then(|| health::recheck(app_data, pack_id)).flatten() {
            Some(Health::Damaged(reason)) => damaged(&reason),
            _ => ToolError::unavailable(error),
        }
    })
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
        health::forget(&folder);
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
    use crate::kit::contracts::{ErrorKind, JobOutcome, OutcomeState};

    const APP: &str = "1.0.14";
    const MODEL: &str = "tessdata/hin.traineddata";

    struct Setup {
        app_data: PathBuf,
        fixture: Fixture,
        catalog: super::super::Catalog,
        key: TestKey,
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
            Self { app_data, fixture, catalog, key }
        }

        fn list(&self) -> PackList {
            list_packs_in(&self.catalog, &self.app_data, &current_target(), APP)
        }

        fn resolve(&self, holds: &mut PackHolds) -> Result<ResolvedResource, ToolError> {
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
        assert_eq!(setup.resolve(&mut later), Err(ToolError::unavailable(format!("Resource {MODEL} is unavailable."))));
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

    impl Setup {
        fn file_path(&self, relative: &str) -> PathBuf {
            self.folder().join(&self.fixture.version).join(relative)
        }

        /// Changes one byte of a pack file and keeps its size, so listing alone cannot tell.
        fn change(&self, relative: &str) -> Vec<u8> {
            let original = std::fs::read(self.file_path(relative)).unwrap();
            let mut changed = original.clone();
            changed[0] ^= 1;
            std::fs::write(self.file_path(relative), changed).unwrap();
            original
        }

        /// The pack file the setup installed.
        fn pack_file(&self) -> Vec<u8> {
            std::fs::read(self.app_data.parent().unwrap().join("pack.toolboxpack")).unwrap()
        }

        fn status(&self) -> PackStatus {
            self.list().packs[0].status
        }

        /// Installs the pack file again, as Repair from file does.
        fn repair_from(&self, bytes: &[u8]) -> Result<(), PackInstallError> {
            let file = self.app_data.parent().unwrap().join("repair.toolboxpack");
            std::fs::write(&file, bytes).unwrap();
            let target = current_target();
            install::install(&file, &InstallContext { catalog: &self.catalog, app_data: &self.app_data, target: &target, app_version: APP, public_key: &self.key.public_key(), limits: PackLimits::DEFAULT, cancel: None })
        }
    }

    fn damaged(pack_id: &str) -> impl Fn(&ToolError) -> bool + '_ {
        move |error| error.kind == ErrorKind::Unavailable
            && error.pointer == Some(OutcomePointer::Pack { pack_id: pack_id.to_string() })
            && error.message.starts_with(&format!("The {pack_id} pack is damaged."))
            && error.message.ends_with("Repair it in Settings, Packs.")
    }

    #[test]
    fn a_file_changed_after_install_marks_the_pack_damaged_at_first_use_and_the_result_is_kept_for_the_session() {
        let setup = Setup::new("changed");
        let original = setup.change(MODEL);
        // Listing compares sizes only, so before the pack is used it still looks installed.
        assert_eq!(setup.status(), PackStatus::Installed);

        let mut run = PackHolds::default();
        let error = setup.resolve(&mut run).unwrap_err();
        assert!(damaged(&setup.fixture.id)(&error), "{error:?}");
        assert!(error.message.contains("ChecksumMismatch"), "{error:?}");
        assert_eq!(holders(&setup.app_data, &setup.fixture.id), 0, "a damaged pack is not held");
        assert_eq!(setup.status(), PackStatus::Damaged);

        // The next resolution reads no files: with the pack moved away and its file restored, the
        // kept result still answers.
        std::fs::write(setup.file_path(MODEL), &original).unwrap();
        let away = setup.app_data.join("away");
        std::fs::rename(setup.folder(), &away).unwrap();
        assert_eq!(setup.resolve(&mut run).unwrap_err(), error);
        std::fs::rename(&away, setup.folder()).unwrap();
        assert_eq!(setup.resolve(&mut run).unwrap_err(), error);
        assert_eq!(setup.status(), PackStatus::Damaged);
    }

    #[test]
    fn a_verified_pack_is_kept_verified_and_a_file_that_later_fails_marks_it_damaged() {
        let setup = Setup::new("later");
        let mut run = PackHolds::default();
        assert_eq!(setup.resolve(&mut run).unwrap().source, ResourceSource::InstalledPack);
        drop(run);
        setup.change(MODEL);
        let mut run = PackHolds::default();
        assert!(damaged(&setup.fixture.id)(&setup.resolve(&mut run).unwrap_err()));
        assert_eq!(holders(&setup.app_data, &setup.fixture.id), 0);
        assert_eq!(setup.status(), PackStatus::Damaged);
    }

    #[test]
    fn missing_files_a_changed_manifest_or_record_each_mark_the_pack_damaged() {
        let cases: [(&str, fn(&Setup)); 4] = [
            ("missing model", |setup| std::fs::remove_file(setup.file_path(MODEL)).unwrap()),
            ("missing manifest", |setup| std::fs::remove_file(setup.file_path("manifest.json")).unwrap()),
            ("changed manifest", |setup| { setup.change("manifest.json"); }),
            ("unreadable record", |setup| std::fs::write(setup.folder().join("installed.json"), b"{").unwrap()),
        ];
        for (case, damage) in cases {
            let setup = Setup::new(&format!("damage-{}", case.replace(' ', "-")));
            damage(&setup);
            let error = setup.resolve(&mut PackHolds::default()).unwrap_err();
            assert!(damaged(&setup.fixture.id)(&error), "{case}: {error:?}");
            assert_eq!(setup.status(), PackStatus::Damaged, "{case}");
        }
    }

    #[test]
    fn a_damaged_pack_is_never_resolved_from_an_override_or_path_and_every_file_fails() {
        let setup = Setup::new("no-fallback");
        setup.change(MODEL);
        let elsewhere = setup.app_data.parent().unwrap().join("elsewhere");
        std::fs::create_dir_all(elsewhere.join("tessdata")).unwrap();
        let development_override = elsewhere.join(MODEL);
        std::fs::write(&development_override, b"an unverified model").unwrap();
        std::fs::write(elsewhere.join("hin.traineddata"), b"a model on PATH").unwrap();

        let outcomes: Vec<JobOutcome> = {
            let _env = crate::kit::lock_process_env();
            let original = std::env::var_os("PATH");
            let mut search = vec![elsewhere.clone(), elsewhere.join("tessdata")];
            search.extend(original.iter().flat_map(std::env::split_paths));
            std::env::set_var("PATH", std::env::join_paths(search).unwrap());
            let mut run = PackHolds::default();
            // Each input the run would process with the resource fails on its own.
            let outcomes = ["first.pdf", "second.pdf"].into_iter().map(|input| {
                match resolve_pack_resource(MODEL, &setup.app_data, &setup.fixture.id, None, Some(development_override.clone()), &mut run) {
                    Ok(resource) => panic!("{input} resolved {resource:?}"),
                    Err(error) => JobOutcome::failure(PathBuf::from(input), error),
                }
            }).collect();
            match original { Some(value) => std::env::set_var("PATH", value), None => std::env::remove_var("PATH") }
            outcomes
        };
        assert_eq!(outcomes.len(), 2);
        for outcome in &outcomes {
            let OutcomeState::Failed { output_paths, error } = &outcome.state else { panic!("{outcome:?}") };
            assert!(output_paths.is_empty());
            assert!(damaged(&setup.fixture.id)(error), "{error:?}");
        }
        assert_eq!(holders(&setup.app_data, &setup.fixture.id), 0);
    }

    #[test]
    fn repair_from_a_pack_file_restores_a_verified_pack_and_a_failed_repair_leaves_it_damaged() {
        let setup = Setup::new("repair-file");
        let original = std::fs::read(setup.file_path(MODEL)).unwrap();
        setup.change(MODEL);
        assert!(setup.resolve(&mut PackHolds::default()).is_err());

        // A pack file that fails verification changes nothing.
        let mut corrupt = setup.pack_file();
        let middle = corrupt.len() / 2;
        corrupt[middle] ^= 0xff;
        assert!(setup.repair_from(&corrupt).is_err());
        assert!(setup.repair_from(&setup.fixture.file(&TestKey::new(2))).is_err(), "signed by another key");
        assert_eq!(setup.status(), PackStatus::Damaged);
        assert!(damaged(&setup.fixture.id)(&setup.resolve(&mut PackHolds::default()).unwrap_err()));
        assert!(!setup.app_data.join("pack-staging").exists(), "a failed repair leaves no staging files");

        setup.repair_from(&setup.pack_file()).unwrap();
        assert_eq!(setup.status(), PackStatus::Installed);
        assert!(!setup.app_data.join("pack-staging").exists());
        let mut run = PackHolds::default();
        let resolved = setup.resolve(&mut run).unwrap();
        assert_eq!((resolved.source, std::fs::read(&resolved.path).unwrap()), (ResourceSource::InstalledPack, original));
    }

    #[test]
    fn uninstalling_a_damaged_pack_forgets_it_and_a_fresh_install_is_checked_again() {
        let setup = Setup::new("damaged-uninstall");
        setup.change(MODEL);
        assert!(setup.resolve(&mut PackHolds::default()).is_err());
        setup.uninstall().unwrap();
        assert_eq!(setup.status(), PackStatus::NotInstalled);
        setup.repair_from(&setup.pack_file()).unwrap();
        assert_eq!(setup.resolve(&mut PackHolds::default()).unwrap().source, ResourceSource::InstalledPack);
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
