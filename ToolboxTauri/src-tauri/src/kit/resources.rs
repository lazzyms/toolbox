use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceManifest {
    pub version: u32,
    pub architecture: String,
    pub resources: std::collections::BTreeMap<String, ResourceSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSpec {
    pub path: PathBuf,
    #[serde(default)]
    pub version: String,
    pub sha256: String,
    pub license: String,
    #[serde(default)]
    pub source_url: String,
    pub max_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceSource { Bundled, DevelopmentOverride, Path }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedResource { pub path: PathBuf, pub source: ResourceSource }

pub fn application_resource_root() -> Option<PathBuf> {
    if let Some(root) = std::env::var_os("TOOLBOX_RESOURCE_ROOT") { return Some(PathBuf::from(root)); }
    resource_root_from_runtime()
}

#[cfg(test)]
fn resource_root_from_runtime() -> Option<PathBuf> {
    Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources"))
}

#[cfg(not(test))]
fn resource_root_from_runtime() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let parent = executable.parent()?;
    if parent.file_name().and_then(|name| name.to_str()) == Some("MacOS") { parent.parent().map(|root| root.join("Resources")) } else { Some(parent.to_path_buf()) }
}

pub(crate) struct Helper {
    name: &'static str,
    override_var: &'static str,
    bundled_dir: &'static str,
    probe: &'static str,
    unavailable: &'static str,
}

pub(crate) const PDF_RENDERER: Helper = Helper {
    name: "pdftoppm",
    override_var: "TOOLBOX_PDFTOPPM_PATH",
    bundled_dir: "pdf-bin",
    probe: "-h",
    unavailable: "pdftoppm is required for PDF previews. Set TOOLBOX_PDFTOPPM_PATH or add it to PATH.",
};

pub(crate) const QPDF: Helper = Helper {
    name: "qpdf",
    override_var: "TOOLBOX_QPDF_PATH",
    bundled_dir: "qpdf-bin",
    probe: "--version",
    unavailable: "This PDF tool is unavailable because a helper it needs is missing from this copy of Toolbox. Reinstall Toolbox to restore it.",
};

struct HelperLocations {
    development_override: Option<PathBuf>,
    executable_dir: Option<PathBuf>,
    resource_root: Option<PathBuf>,
    system_path: Option<std::ffi::OsString>,
}

impl HelperLocations {
    fn of_process(helper: &Helper) -> Self {
        Self {
            development_override: std::env::var_os(helper.override_var).filter(|path| !path.is_empty()).map(PathBuf::from),
            executable_dir: std::env::current_exe().ok().and_then(|executable| executable.parent().map(Path::to_path_buf)),
            resource_root: application_resource_root(),
            system_path: std::env::var_os("PATH"),
        }
    }
}

pub(crate) fn resolve_helper(helper: &Helper) -> Result<PathBuf, String> {
    locate_helper(helper, &HelperLocations::of_process(helper))
}

fn locate_helper(helper: &Helper, locations: &HelperLocations) -> Result<PathBuf, String> {
    if let Some(path) = &locations.development_override {
        if path.is_file() { return Ok(path.clone()); }
        return Err(format!("{} does not point to a file.", helper.override_var));
    }
    let executable = if cfg!(windows) { format!("{}.exe", helper.name) } else { helper.name.to_string() };
    let beside_executable = locations.executable_dir.iter().map(|dir| dir.join(&executable));
    let bundled = locations.resource_root.iter()
        .flat_map(|root| [root.join(helper.bundled_dir).join(&executable), root.join("resources").join(&executable), root.join(&executable)]);
    let on_path = locations.system_path.iter().flat_map(std::env::split_paths).map(|dir| dir.join(&executable));
    beside_executable.chain(bundled).chain(on_path)
        .find(|path| path.is_file() && {
            let mut command = Command::new(path);
            command.arg(helper.probe);
            crate::kit::pdf::helper_available(command)
        })
        .ok_or_else(|| helper.unavailable.to_string())
}

pub fn resolve(name: &str, bundled_root: &Path, override_var: &str, path_name: &str) -> Result<ResolvedResource, String> {
    if let Some(resource) = resolve_bundled_or_override(name, bundled_root, override_var)? { return Ok(resource); }
    find_on_path(path_name).map(|path| ResolvedResource { path, source: ResourceSource::Path })
        .ok_or_else(|| format!("Resource {name} is unavailable."))
}

pub fn resolve_bundled_or_override(name: &str, bundled_root: &Path, override_var: &str) -> Result<Option<ResolvedResource>, String> {
    let manifest_path = bundled_root.join("manifest.json");
    if manifest_path.is_file() {
        let manifest: ResourceManifest = serde_json::from_slice(&fs::read(&manifest_path).map_err(|e| format!("Cannot read resource manifest: {e}"))?)
            .map_err(|e| format!("Invalid resource manifest: {e}"))?;
        let spec = manifest.resources.get(name).ok_or_else(|| format!("Resource {name} is missing from the bundled manifest."))?;
        if manifest.version != 1 || (manifest.architecture != "all" && manifest.architecture != std::env::consts::ARCH) { return Err("Bundled resource manifest does not match this application.".to_string()); }
        let path = bundled_root.join(&spec.path);
        verify(&path, spec)?;
        return Ok(Some(ResolvedResource { path, source: ResourceSource::Bundled }));
    }
    if let Ok(path) = std::env::var(override_var) {
        let path = PathBuf::from(path);
        if path.is_file() { return Ok(Some(ResolvedResource { path, source: ResourceSource::DevelopmentOverride })); }
        return Err(format!("Development resource override {override_var} does not point to a file."));
    }
    Ok(None)
}

fn verify(path: &Path, spec: &ResourceSpec) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|e| format!("Bundled resource is unavailable: {e}"))?;
    if spec.version.trim().is_empty() { return Err("Bundled resource has no version metadata.".to_string()); }
    if spec.license.trim().is_empty() { return Err("Bundled resource has no license metadata.".to_string()); }
    if bytes.len() as u64 > spec.max_bytes { return Err("Bundled resource exceeds its declared size limit.".to_string()); }
    let digest = digest_hex(&bytes);
    if digest != spec.sha256 { return Err("Bundled resource checksum does not match its manifest.".to_string()); }
    Ok(())
}

fn digest_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?.to_string_lossy().split(if cfg!(windows) { ';' } else { ':' })
        .map(Path::new).map(|dir| dir.join(name)).find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn root() -> PathBuf { std::env::temp_dir().join(format!("toolbox_resources_{}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos())) }

    #[test]
    fn bundled_resource_wins_and_is_verified() {
        let root = root();
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::write(root.join("bin/adapter"), b"bundled").unwrap();
        let checksum = digest_hex(b"bundled");
        let mut resources = BTreeMap::new();
        resources.insert("adapter".to_string(), ResourceSpec { path: PathBuf::from("bin/adapter"), version: "test".to_string(), sha256: checksum, license: "MIT".to_string(), source_url: "https://example.test/adapter".to_string(), max_bytes: 100 });
        fs::write(root.join("manifest.json"), serde_json::to_vec(&ResourceManifest { version: 1, architecture: std::env::consts::ARCH.to_string(), resources }).unwrap()).unwrap();
        let result = resolve("adapter", &root, "TOOLBOX_TEST_OVERRIDE", "adapter").unwrap();
        assert_eq!(result.source, ResourceSource::Bundled);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn checksum_failure_does_not_fall_back() {
        let root = root();
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("bin"), b"changed").unwrap();
        let mut resources = BTreeMap::new();
        resources.insert("adapter".to_string(), ResourceSpec { path: PathBuf::from("bin"), version: "test".to_string(), sha256: "bad".to_string(), license: "MIT".to_string(), source_url: "https://example.test/adapter".to_string(), max_bytes: 100 });
        fs::write(root.join("manifest.json"), serde_json::to_vec(&ResourceManifest { version: 1, architecture: std::env::consts::ARCH.to_string(), resources }).unwrap()).unwrap();
        assert!(resolve("adapter", &root, "TOOLBOX_TEST_OVERRIDE", "adapter").is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    fn fake_executable(path: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        path.to_path_buf()
    }

    #[cfg(unix)]
    #[test]
    fn qpdf_lookup_prefers_override_then_executable_dir_then_bundled_resources_then_system_path() {
        let root = root();
        let development_override = fake_executable(&root.join("override/qpdf"));
        let beside_executable = fake_executable(&root.join("app/qpdf"));
        let bundled = fake_executable(&root.join("Resources/qpdf-bin/qpdf"));
        let on_path = fake_executable(&root.join("system/qpdf"));
        let mut locations = HelperLocations {
            development_override: Some(development_override.clone()),
            executable_dir: Some(root.join("app")),
            resource_root: Some(root.join("Resources")),
            system_path: Some(std::env::join_paths([root.join("empty"), root.join("system")]).unwrap()),
        };

        assert_eq!(locate_helper(&QPDF, &locations), Ok(development_override));
        locations.development_override = None;
        assert_eq!(locate_helper(&QPDF, &locations), Ok(beside_executable.clone()));
        fs::remove_file(&beside_executable).unwrap();
        assert_eq!(locate_helper(&QPDF, &locations), Ok(bundled.clone()));
        fs::remove_file(&bundled).unwrap();
        assert_eq!(locate_helper(&QPDF, &locations), Ok(on_path.clone()));
        fs::remove_file(&on_path).unwrap();
        let missing = locate_helper(&QPDF, &locations).unwrap_err();
        assert_eq!(missing, QPDF.unavailable);
        assert!(!missing.contains("TOOLBOX_") && !missing.contains("PATH"), "{missing}");
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn qpdf_lookup_finds_bundled_resources_copy_and_skips_helpers_that_do_not_run() {
        use std::os::unix::fs::PermissionsExt;
        let root = root();
        let broken = fake_executable(&root.join("Resources/qpdf-bin/qpdf"));
        fs::write(&broken, b"#!/bin/sh\nexit 1\n").unwrap();
        fs::set_permissions(&broken, fs::Permissions::from_mode(0o755)).unwrap();
        let resources_subdir = fake_executable(&root.join("Resources/resources/qpdf"));
        let locations = HelperLocations { development_override: None, executable_dir: Some(root.join("app")), resource_root: Some(root.join("Resources")), system_path: None };
        assert_eq!(locate_helper(&QPDF, &locations), Ok(resources_subdir));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn qpdf_override_that_is_not_a_file_does_not_fall_back() {
        let root = root();
        let locations = HelperLocations { development_override: Some(root.join("missing/qpdf")), executable_dir: None, resource_root: None, system_path: std::env::var_os("PATH") };
        assert!(locate_helper(&QPDF, &locations).is_err());
    }

    fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
        let mut parts = text.trim().splitn(3, '.').map(|part| part.parse::<u32>().ok());
        Some((parts.next()??, parts.next()??, parts.next()??))
    }

    #[test]
    fn resolved_qpdf_meets_the_pinned_minimum_version() {
        let Ok(qpdf) = resolve_helper(&QPDF) else {
            if std::env::var_os("TOOLBOX_REQUIRE_QPDF").is_some() { panic!("qpdf is required for this test run"); }
            eprintln!("skipping: qpdf not available");
            return;
        };
        let pin: serde_json::Value = serde_json::from_str(include_str!("../../../qpdf-pin.json")).unwrap();
        let minimum = pin["minimumVersion"].as_str().and_then(parse_version).expect("qpdf-pin.json has a minimumVersion");
        let output = Command::new(&qpdf).arg("--version").output().unwrap();
        let reported = String::from_utf8_lossy(&output.stdout);
        let version = reported.lines().find_map(|line| line.strip_prefix("qpdf version ")).and_then(parse_version)
            .unwrap_or_else(|| panic!("{} did not report a qpdf version: {reported}", qpdf.display()));
        assert!(version >= minimum, "{} reports {version:?}, below the pinned minimum {minimum:?}", qpdf.display());
    }

    #[test]
    fn bundled_watermark_font_manifest_is_pinned_and_verified() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/fonts");
        let result = resolve("watermarkFont", &root, "TOOLBOX_WATERMARK_FONT_PATH", "DejaVuSans.ttf").unwrap();
        assert_eq!(result.source, ResourceSource::Bundled);
        assert_eq!(result.path.file_name().and_then(|name| name.to_str()), Some("DejaVuSans.ttf"));
    }
}
