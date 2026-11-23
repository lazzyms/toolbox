//! The only place tests change process environment.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

const RESOURCE_ROOT: &str = "TOOLBOX_RESOURCE_ROOT";

static PROCESS_ENV_LOCK: Mutex<()> = Mutex::new(());

/// Serializes tests that read process environment. A failed test must not fail every later one,
/// so a poisoned lock is still handed out.
pub(crate) fn lock_process_env() -> MutexGuard<'static, ()> {
    PROCESS_ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Holds the process environment lock, applies overrides as they are named and restores every
/// variable it touched, last change first, before the lock is released.
///
/// A staged resource bundle outranks `TOOLBOX_*_PATH` helper overrides, so the first helper
/// override without a `TOOLBOX_RESOURCE_ROOT` override points the root at an empty folder.
pub(crate) struct EnvOverride {
    saved: Vec<(String, Option<OsString>)>,
    empty_root: Option<PathBuf>,
    _lock: MutexGuard<'static, ()>,
}

impl EnvOverride {
    pub(crate) fn new() -> Self {
        Self { saved: Vec::new(), empty_root: None, _lock: lock_process_env() }
    }

    pub(crate) fn set(mut self, key: &str, value: impl AsRef<OsStr>) -> Self {
        self.apply(key, Some(value.as_ref()));
        self
    }

    pub(crate) fn remove(mut self, key: &str) -> Self {
        self.apply(key, None);
        self
    }

    fn apply(&mut self, key: &str, value: Option<&OsStr>) {
        if key.starts_with("TOOLBOX_") && key.ends_with("_PATH") && !self.touched(RESOURCE_ROOT) {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let root = std::env::temp_dir().join(format!("toolbox-empty-resource-root-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir_all(&root).unwrap();
            self.write(RESOURCE_ROOT, Some(root.as_os_str()));
            self.empty_root = Some(root);
        }
        self.write(key, value);
    }

    fn touched(&self, key: &str) -> bool {
        self.saved.iter().any(|(saved, _)| saved == key)
    }

    fn write(&mut self, key: &str, value: Option<&OsStr>) {
        if !self.touched(key) {
            self.saved.push((key.to_string(), std::env::var_os(key)));
        }
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
}

impl Drop for EnvOverride {
    fn drop(&mut self) {
        for (key, value) in self.saved.drain(..).rev() {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
        if let Some(root) = self.empty_root.take() {
            let _ = std::fs::remove_dir_all(root);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::resources::{application_resource_root, resolve_bundled_or_override, ResourceSource};

    #[test]
    fn a_helper_override_alone_hides_every_bundle_and_is_undone_on_drop() {
        let env = EnvOverride::new();
        let outer = std::env::var_os(RESOURCE_ROOT);
        let helper = std::env::current_exe().unwrap();
        let env = env.set("TOOLBOX_TESSERACT_PATH", &helper);

        let root = application_resource_root().unwrap();
        assert!(root.is_dir() && root.read_dir().unwrap().next().is_none(), "the root is an empty folder: {root:?}");
        let resolved = resolve_bundled_or_override("tesseract", &root.join("ocr"), "TOOLBOX_TESSERACT_PATH").unwrap().unwrap();
        assert_eq!((resolved.source, resolved.path), (ResourceSource::DevelopmentOverride, helper));

        drop(env);
        let _lock = lock_process_env();
        assert_eq!(std::env::var_os(RESOURCE_ROOT), outer);
        assert!(!root.exists(), "the empty folder is removed");
    }

    #[test]
    fn a_root_the_test_names_is_kept() {
        let env = EnvOverride::new().set(RESOURCE_ROOT, "/named-root").set("TOOLBOX_TESSERACT_PATH", "x");
        assert_eq!(application_resource_root().unwrap(), PathBuf::from("/named-root"));
        drop(env);
    }
}
