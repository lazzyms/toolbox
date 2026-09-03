use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

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
    pub sha256: String,
    pub license: String,
    pub max_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceSource { Bundled, DevelopmentOverride, Path }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedResource { pub path: PathBuf, pub source: ResourceSource }

pub fn resolve(name: &str, bundled_root: &Path, override_var: &str, path_name: &str) -> Result<ResolvedResource, String> {
    let manifest_path = bundled_root.join("manifest.json");
    if manifest_path.is_file() {
        let manifest: ResourceManifest = serde_json::from_slice(&fs::read(&manifest_path).map_err(|e| format!("Cannot read resource manifest: {e}"))?)
            .map_err(|e| format!("Invalid resource manifest: {e}"))?;
        let spec = manifest.resources.get(name).ok_or_else(|| format!("Resource {name} is missing from the bundled manifest."))?;
        if manifest.version != 1 || manifest.architecture != std::env::consts::ARCH { return Err("Bundled resource manifest does not match this application.".to_string()); }
        let path = bundled_root.join(&spec.path);
        verify(&path, spec)?;
        return Ok(ResolvedResource { path, source: ResourceSource::Bundled });
    }
    if let Ok(path) = std::env::var(override_var) {
        let path = PathBuf::from(path);
        if path.is_file() { return Ok(ResolvedResource { path, source: ResourceSource::DevelopmentOverride }); }
        return Err(format!("Development resource override {override_var} does not point to a file."));
    }
    find_on_path(path_name).map(|path| ResolvedResource { path, source: ResourceSource::Path })
        .ok_or_else(|| format!("Resource {name} is unavailable."))
}

fn verify(path: &Path, spec: &ResourceSpec) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|e| format!("Bundled resource is unavailable: {e}"))?;
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
        resources.insert("adapter".to_string(), ResourceSpec { path: PathBuf::from("bin/adapter"), sha256: checksum, license: "MIT".to_string(), max_bytes: 100 });
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
        resources.insert("adapter".to_string(), ResourceSpec { path: PathBuf::from("bin"), sha256: "bad".to_string(), license: "MIT".to_string(), max_bytes: 100 });
        fs::write(root.join("manifest.json"), serde_json::to_vec(&ResourceManifest { version: 1, architecture: std::env::consts::ARCH.to_string(), resources }).unwrap()).unwrap();
        assert!(resolve("adapter", &root, "TOOLBOX_TEST_OVERRIDE", "adapter").is_err());
        let _ = fs::remove_dir_all(root);
    }
}
