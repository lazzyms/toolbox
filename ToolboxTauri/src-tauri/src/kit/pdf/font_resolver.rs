#![cfg_attr(not(test), allow(dead_code))]

//! Font resolution for the built-in document renderer.
//!
//! For each requested family the resolver returns, in order: a font embedded in the
//! source, an installed system font with that family name, a bundled metric-compatible
//! fallback, or the bundled default. Every result carries a report entry naming the
//! requested family, the font used and its source.
//!
//! The bundled fonts, their substitutes and the fallback install-size budget are listed
//! under `fallbackFonts` in `resources/fonts/manifest.json`. Discovery verifies every
//! listed face against its manifest checksum before the resolver can use it.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::kit::contracts::ToolError;
use crate::kit::pdf::unicode_font::UnicodeFont;
use crate::kit::resources::{self, ResourceSpec};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FontStyle { Regular, Bold, Italic, BoldItalic }

impl FontStyle {
    pub fn new(bold: bool, italic: bool) -> Self {
        match (bold, italic) {
            (false, false) => Self::Regular,
            (true, false) => Self::Bold,
            (false, true) => Self::Italic,
            (true, true) => Self::BoldItalic,
        }
    }

    fn manifest_key(key: &str) -> Option<Self> {
        match key {
            "regular" => Some(Self::Regular),
            "bold" => Some(Self::Bold),
            "italic" => Some(Self::Italic),
            "boldItalic" => Some(Self::BoldItalic),
            _ => None,
        }
    }

    /// The faces to try for this style, closest first. A face of another style is used
    /// as is and the renderer synthesizes the missing weight or slant.
    fn preference(self) -> [Self; 4] {
        match self {
            Self::Regular => [Self::Regular, Self::Bold, Self::Italic, Self::BoldItalic],
            Self::Bold => [Self::Bold, Self::Regular, Self::BoldItalic, Self::Italic],
            Self::Italic => [Self::Italic, Self::Regular, Self::BoldItalic, Self::Bold],
            Self::BoldItalic => [Self::BoldItalic, Self::Bold, Self::Italic, Self::Regular],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum FontSource {
    #[serde(rename = "embedded")]
    Embedded,
    #[serde(rename = "system")]
    System,
    #[serde(rename = "bundled fallback")]
    BundledFallback,
    #[serde(rename = "bundled default")]
    BundledDefault,
}

/// One entry of the per-file font detail: what was asked for, what was used and where it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontReport {
    pub requested_family: String,
    pub requested_style: FontStyle,
    pub font_used: String,
    pub style_used: FontStyle,
    pub source: FontSource,
}

pub struct ResolvedFont {
    pub report: FontReport,
    pub data: Vec<u8>,
}

/// A font program carried inside the source document.
pub struct EmbeddedFont {
    pub family: String,
    pub style: FontStyle,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemFace {
    pub family: String,
    pub style: FontStyle,
    pub path: PathBuf,
}

/// Finds installed fonts by family name. Injected so tests do not depend on the machine's fonts.
pub trait SystemFontLookup {
    fn faces(&self, family: &str) -> Vec<SystemFace>;
}

fn normalized(family: &str) -> String {
    family.trim().trim_matches(|c| c == '"' || c == '\'').split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn usable(data: &[u8]) -> bool {
    UnicodeFont::parse(data.to_vec()).is_ok()
}

fn closest<T>(requested: FontStyle, mut style_of: impl FnMut(FontStyle) -> Option<T>) -> Option<(FontStyle, T)> {
    requested.preference().into_iter().find_map(|style| style_of(style).map(|found| (style, found)))
}

// ---------------------------------------------------------------------------
// Bundled fonts
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FontManifest {
    version: u32,
    architecture: String,
    resources: BTreeMap<String, ResourceSpec>,
    fallback_fonts: FallbackPolicy,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FallbackPolicy {
    budget_bytes: u64,
    families: Vec<FamilyPolicy>,
    default: FamilyPolicy,
}

#[derive(Deserialize)]
struct FamilyPolicy {
    family: String,
    #[serde(default)]
    substitutes: Vec<String>,
    faces: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
struct BundledFamily {
    family: String,
    names: Vec<String>,
    faces: BTreeMap<FontStyle, PathBuf>,
}

/// The bundled fallback and default fonts, each verified against the manifest at discovery.
#[derive(Debug, Clone)]
pub struct BundledFonts {
    families: Vec<BundledFamily>,
    default: BundledFamily,
    budget_bytes: u64,
    fallback_bytes: u64,
}

fn damaged(detail: impl std::fmt::Display) -> ToolError {
    ToolError::unavailable(format!("The bundled fonts in this copy of Toolbox are missing or damaged ({detail}). Reinstall Toolbox to restore them."))
}

impl BundledFonts {
    /// Discovers the bundled fonts in the installed application's resources.
    pub fn discover_installed() -> Result<Self, ToolError> {
        let root = resources::application_resource_root().ok_or_else(|| damaged("no resource folder"))?;
        let fonts = [root.join("resources").join("fonts"), root.join("fonts")].into_iter()
            .find(|candidate| candidate.join("manifest.json").is_file())
            .ok_or_else(|| damaged("no font manifest"))?;
        Self::discover(&fonts)
    }

    /// Reads `manifest.json` in `fonts_root`, verifies every fallback and default face against its
    /// checksum, checks that it is a usable TrueType font, and enforces the fallback size budget.
    pub fn discover(fonts_root: &Path) -> Result<Self, ToolError> {
        let bytes = fs::read(fonts_root.join("manifest.json")).map_err(|error| damaged(format!("cannot read the font manifest: {error}")))?;
        let manifest: FontManifest = serde_json::from_slice(&bytes).map_err(|error| damaged(format!("invalid font manifest: {error}")))?;
        resources::ensure_manifest_matches_application(manifest.version, &manifest.architecture).map_err(damaged)?;
        let family = |policy: &FamilyPolicy| -> Result<BundledFamily, ToolError> {
            let mut faces = BTreeMap::new();
            for (style, key) in &policy.faces {
                let style = FontStyle::manifest_key(style).ok_or_else(|| damaged(format!("{} has an unknown style {style}", policy.family)))?;
                let spec = manifest.resources.get(key).ok_or_else(|| damaged(format!("{key} is not in the font manifest")))?;
                let path = resources::verify_bundled(fonts_root, spec).map_err(|error| damaged(format!("{key}: {error}")))?;
                let data = fs::read(&path).map_err(|error| damaged(format!("{key}: {error}")))?;
                if !usable(&data) { return Err(damaged(format!("{key} is not a usable TrueType font"))); }
                faces.insert(style, path);
            }
            if !faces.contains_key(&FontStyle::Regular) { return Err(damaged(format!("{} has no regular face", policy.family))); }
            let names = std::iter::once(&policy.family).chain(&policy.substitutes).map(|name| normalized(name)).collect();
            Ok(BundledFamily { family: policy.family.clone(), names, faces })
        };
        let families = manifest.fallback_fonts.families.iter().map(family).collect::<Result<Vec<_>, _>>()?;
        let default = family(&manifest.fallback_fonts.default)?;
        let mut paths = families.iter().flat_map(|family| family.faces.values()).collect::<Vec<_>>();
        paths.sort();
        paths.dedup();
        let fallback_bytes = paths.iter().map(|path| fs::metadata(path).map(|metadata| metadata.len()).unwrap_or(0)).sum();
        let budget_bytes = manifest.fallback_fonts.budget_bytes;
        if fallback_bytes > budget_bytes {
            return Err(damaged(format!("the fallback fonts take {fallback_bytes} bytes, over the {budget_bytes}-byte budget")));
        }
        Ok(Self { families, default, budget_bytes, fallback_bytes })
    }

    pub fn budget_bytes(&self) -> u64 { self.budget_bytes }
    pub fn fallback_bytes(&self) -> u64 { self.fallback_bytes }
    pub fn fallback_families(&self) -> impl Iterator<Item = &str> { self.families.iter().map(|family| family.family.as_str()) }
    pub fn default_family(&self) -> &str { &self.default.family }

    fn fallback_for(&self, family: &str) -> Option<&BundledFamily> {
        let wanted = normalized(family);
        self.families.iter().find(|candidate| candidate.names.contains(&wanted))
    }
}

// ---------------------------------------------------------------------------
// Installed system fonts
// ---------------------------------------------------------------------------

/// Installed TrueType fonts, indexed once per process from the platform font folders.
/// Collections (.ttc) and CFF-flavoured OpenType fonts are not indexed, because the
/// PDF writer embeds TrueType outlines only.
pub struct InstalledSystemFonts {
    folders: Vec<PathBuf>,
    index: OnceLock<BTreeMap<String, Vec<SystemFace>>>,
}

impl InstalledSystemFonts {
    pub fn new() -> Self { Self::in_folders(platform_font_folders()) }

    pub fn in_folders(folders: Vec<PathBuf>) -> Self { Self { folders, index: OnceLock::new() } }

    fn index(&self) -> &BTreeMap<String, Vec<SystemFace>> {
        self.index.get_or_init(|| {
            let mut index: BTreeMap<String, Vec<SystemFace>> = BTreeMap::new();
            let mut files = Vec::new();
            for folder in &self.folders { font_files(folder, 4, &mut files); }
            files.sort();
            for path in files {
                if let Some((family, style)) = describe_font_file(&path) {
                    index.entry(normalized(&family)).or_default().push(SystemFace { family, style, path });
                }
            }
            index
        })
    }
}

impl Default for InstalledSystemFonts {
    fn default() -> Self { Self::new() }
}

impl SystemFontLookup for InstalledSystemFonts {
    fn faces(&self, family: &str) -> Vec<SystemFace> {
        self.index().get(&normalized(family)).cloned().unwrap_or_default()
    }
}

fn platform_font_folders() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        let mut folders = vec![PathBuf::from("/System/Library/Fonts"), PathBuf::from("/Library/Fonts")];
        folders.extend(home.map(|home| home.join("Library/Fonts")));
        folders
    } else if cfg!(windows) {
        let mut folders = Vec::new();
        folders.extend(std::env::var_os("WINDIR").map(|windows| PathBuf::from(windows).join("Fonts")));
        folders.extend(std::env::var_os("LOCALAPPDATA").map(|local| PathBuf::from(local).join("Microsoft").join("Windows").join("Fonts")));
        folders
    } else {
        let mut folders = vec![PathBuf::from("/usr/share/fonts"), PathBuf::from("/usr/local/share/fonts")];
        folders.extend(home.iter().flat_map(|home| [home.join(".local/share/fonts"), home.join(".fonts")]));
        folders
    }
}

fn font_files(folder: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(folder) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() && depth > 0 {
            font_files(&path, depth - 1, found);
        } else if kind.is_file() && path.extension().and_then(|extension| extension.to_str()).is_some_and(|extension| extension.eq_ignore_ascii_case("ttf")) {
            found.push(path);
        }
    }
}

/// Reads only the table directory, `name` and `head` of a TrueType file to learn its family and style.
fn describe_font_file(path: &Path) -> Option<(String, FontStyle)> {
    let mut file = fs::File::open(path).ok()?;
    let mut header = [0_u8; 12];
    file.read_exact(&mut header).ok()?;
    if u32::from_be_bytes([header[0], header[1], header[2], header[3]]) != 0x0001_0000 && &header[..4] != b"true" { return None; }
    let count = u16::from_be_bytes([header[4], header[5]]) as usize;
    let mut directory = vec![0_u8; count * 16];
    file.read_exact(&mut directory).ok()?;
    let mut read_table = |tag: &[u8; 4]| -> Option<Vec<u8>> {
        let record = directory.chunks_exact(16).find(|record| &record[..4] == tag)?;
        let offset = u32::from_be_bytes([record[8], record[9], record[10], record[11]]) as u64;
        let length = u32::from_be_bytes([record[12], record[13], record[14], record[15]]) as usize;
        if length > 4 * 1024 * 1024 { return None; }
        let mut data = vec![0_u8; length];
        file.seek(SeekFrom::Start(offset)).ok()?;
        file.read_exact(&mut data).ok()?;
        Some(data)
    };
    if !directory.chunks_exact(16).any(|record| &record[..4] == b"glyf") { return None; }
    let head = read_table(b"head")?;
    let names = read_table(b"name")?;
    let mac_style = u16::from_be_bytes([*head.get(44)?, *head.get(45)?]);
    let table = rustybuzz::ttf_parser::name::Table::parse(&names)?;
    let family = |id: u16| table.names.into_iter().filter(|name| name.name_id == id && name.is_unicode()).find_map(|name| name.to_string()).filter(|name| !name.trim().is_empty());
    let family = family(rustybuzz::ttf_parser::name_id::TYPOGRAPHIC_FAMILY).or_else(|| family(rustybuzz::ttf_parser::name_id::FAMILY))?;
    Some((family, FontStyle::new(mac_style & 1 != 0, mac_style & 2 != 0)))
}

// ---------------------------------------------------------------------------
// Resolver
// ---------------------------------------------------------------------------

pub struct FontResolver<S: SystemFontLookup = InstalledSystemFonts> {
    bundled: BundledFonts,
    system: S,
}

impl FontResolver<InstalledSystemFonts> {
    pub fn installed() -> Result<Self, ToolError> {
        Ok(Self::new(BundledFonts::discover_installed()?, InstalledSystemFonts::new()))
    }
}

impl<S: SystemFontLookup> FontResolver<S> {
    pub fn new(bundled: BundledFonts, system: S) -> Self { Self { bundled, system } }

    /// Resolves one requested family and style: embedded, then system, then bundled fallback, then bundled default.
    pub fn resolve(&self, family: &str, style: FontStyle, embedded: &[EmbeddedFont]) -> Result<ResolvedFont, ToolError> {
        let report = |font_used: &str, style_used, source| FontReport {
            requested_family: family.trim().to_string(), requested_style: style, font_used: font_used.to_string(), style_used, source,
        };
        let wanted = normalized(family);

        let embedded_match = closest(style, |candidate| embedded.iter().find(|font| font.style == candidate && normalized(&font.family) == wanted && usable(&font.data)));
        if let Some((style_used, font)) = embedded_match {
            return Ok(ResolvedFont { report: report(&font.family, style_used, FontSource::Embedded), data: font.data.clone() });
        }

        if !wanted.is_empty() {
            let installed = self.system.faces(family);
            let system_match = closest(style, |candidate| installed.iter().filter(|face| face.style == candidate).find_map(|face| {
                fs::read(&face.path).ok().filter(|data| usable(data)).map(|data| (face, data))
            }));
            if let Some((style_used, (face, data))) = system_match {
                return Ok(ResolvedFont { report: report(&face.family, style_used, FontSource::System), data });
            }
        }

        let (bundled, source) = match self.bundled.fallback_for(family) {
            Some(fallback) => (fallback, FontSource::BundledFallback),
            None => (&self.bundled.default, FontSource::BundledDefault),
        };
        let (style_used, path) = closest(style, |candidate| bundled.faces.get(&candidate)).ok_or_else(|| damaged(format!("{} has no faces", bundled.family)))?;
        let data = fs::read(path).map_err(|error| damaged(format!("{}: {error}", path.display())))?;
        Ok(ResolvedFont { report: report(&bundled.family, style_used, source), data })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fallback fonts and install-size budget chosen for #589 (shared with the Formatted PDF and EPUB spec).
    const FALLBACK_FAMILIES: [(&str, &[&str]); 5] = [
        ("Liberation Sans", &["Arial", "Helvetica"]),
        ("Liberation Serif", &["Times New Roman", "Times"]),
        ("Liberation Mono", &["Courier New", "Courier"]),
        ("Carlito", &["Calibri"]),
        ("Caladea", &["Cambria"]),
    ];
    const FALLBACK_BUDGET_BYTES: u64 = 6_000_000;
    const FALLBACK_BYTES: u64 = 5_728_224;

    const DEJAVU: &[u8] = include_bytes!("../../../resources/fonts/DejaVuSans.ttf");

    fn fonts_root() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/fonts") }
    fn bundled(file: &str) -> Vec<u8> { fs::read(fonts_root().join(file)).unwrap() }

    #[derive(Default)]
    struct FakeSystem(Vec<SystemFace>);
    impl SystemFontLookup for FakeSystem {
        fn faces(&self, family: &str) -> Vec<SystemFace> {
            self.0.iter().filter(|face| normalized(&face.family) == normalized(family)).cloned().collect()
        }
    }

    fn resolver(system: FakeSystem) -> FontResolver<FakeSystem> {
        FontResolver::new(BundledFonts::discover(&fonts_root()).unwrap(), system)
    }

    fn sandbox(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("toolbox_font_resolver_{}_{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn copy_fonts(name: &str) -> PathBuf {
        let root = sandbox(name);
        for entry in fs::read_dir(fonts_root()).unwrap().flatten() { fs::copy(entry.path(), root.join(entry.file_name())).unwrap(); }
        root
    }

    #[test]
    fn embedded_family_resolves_to_the_embedded_font_before_an_installed_one() {
        let installed = FakeSystem(vec![SystemFace { family: "Brand Sans".into(), style: FontStyle::Regular, path: fonts_root().join("LiberationSans-Regular.ttf") }]);
        let embedded = [EmbeddedFont { family: "Brand Sans".into(), style: FontStyle::Regular, data: DEJAVU.to_vec() }];
        let resolved = resolver(installed).resolve("brand  sans", FontStyle::Regular, &embedded).unwrap();
        assert_eq!(resolved.report, FontReport {
            requested_family: "brand  sans".into(), requested_style: FontStyle::Regular, font_used: "Brand Sans".into(), style_used: FontStyle::Regular, source: FontSource::Embedded,
        });
        assert_eq!(resolved.data, DEJAVU);
        assert_eq!(serde_json::to_value(&resolved.report).unwrap()["source"], "embedded");
    }

    #[test]
    fn unusable_embedded_font_falls_through_to_the_next_source() {
        let embedded = [EmbeddedFont { family: "Arial".into(), style: FontStyle::Regular, data: b"not a font".to_vec() }];
        let resolved = resolver(FakeSystem::default()).resolve("Arial", FontStyle::Regular, &embedded).unwrap();
        assert_eq!(resolved.report.source, FontSource::BundledFallback);
    }

    #[test]
    fn installed_system_family_is_used_before_the_bundled_fallback() {
        let installed = FakeSystem(vec![
            SystemFace { family: "Arial".into(), style: FontStyle::Regular, path: fonts_root().join("DejaVuSans.ttf") },
            SystemFace { family: "Arial".into(), style: FontStyle::Bold, path: fonts_root().join("missing.ttf") },
        ]);
        let resolved = resolver(installed).resolve("Arial", FontStyle::Bold, &[]).unwrap();
        assert_eq!((resolved.report.font_used.as_str(), resolved.report.style_used, resolved.report.source), ("Arial", FontStyle::Regular, FontSource::System));
        assert_eq!(resolved.data, DEJAVU);
        assert_eq!(serde_json::to_value(&resolved.report).unwrap()["source"], "system");
    }

    #[test]
    fn family_neither_embedded_nor_installed_resolves_to_its_metric_compatible_fallback() {
        let resolver = resolver(FakeSystem::default());
        let cases = [
            ("Arial", FontStyle::Regular, "Liberation Sans", FontStyle::Regular, "LiberationSans-Regular.ttf"),
            ("Arial", FontStyle::Bold, "Liberation Sans", FontStyle::Bold, "LiberationSans-Bold.ttf"),
            ("Arial", FontStyle::Italic, "Liberation Sans", FontStyle::Regular, "LiberationSans-Regular.ttf"),
            ("Helvetica", FontStyle::BoldItalic, "Liberation Sans", FontStyle::Bold, "LiberationSans-Bold.ttf"),
            ("Times New Roman", FontStyle::Italic, "Liberation Serif", FontStyle::Italic, "LiberationSerif-Italic.ttf"),
            ("Courier New", FontStyle::Bold, "Liberation Mono", FontStyle::Regular, "LiberationMono-Regular.ttf"),
            ("calibri", FontStyle::BoldItalic, "Carlito", FontStyle::BoldItalic, "Carlito-BoldItalic.ttf"),
            ("Cambria", FontStyle::Bold, "Caladea", FontStyle::Bold, "Caladea-Bold.ttf"),
            ("Carlito", FontStyle::Regular, "Carlito", FontStyle::Regular, "Carlito-Regular.ttf"),
        ];
        for (family, style, used, style_used, file) in cases {
            let resolved = resolver.resolve(family, style, &[]).unwrap();
            assert_eq!(resolved.report, FontReport {
                requested_family: family.into(), requested_style: style, font_used: used.into(), style_used, source: FontSource::BundledFallback,
            }, "{family} {style:?}");
            assert_eq!(resolved.data, bundled(file), "{family} {style:?}");
        }
        let report = resolver.resolve("Arial", FontStyle::Regular, &[]).unwrap().report;
        assert_eq!(serde_json::to_value(&report).unwrap()["source"], "bundled fallback");
    }

    #[test]
    fn unknown_family_resolves_to_the_bundled_default() {
        let resolver = resolver(FakeSystem::default());
        for (family, style) in [("Toolbox Unknown Display", FontStyle::Bold), ("", FontStyle::Regular)] {
            let resolved = resolver.resolve(family, style, &[]).unwrap();
            assert_eq!((resolved.report.font_used.as_str(), resolved.report.style_used, resolved.report.source), ("DejaVu Sans", FontStyle::Regular, FontSource::BundledDefault));
            assert_eq!(resolved.data, DEJAVU);
        }
    }

    #[test]
    fn a_family_that_is_not_embedded_reports_a_substitute_and_its_source() {
        // Uses the real installed fonts, so it asserts only that a substitute and its source are reported.
        let resolver = FontResolver::installed().unwrap();
        let resolved = resolver.resolve("Calibri", FontStyle::Regular, &[]).unwrap();
        assert!(!resolved.report.font_used.trim().is_empty());
        assert_ne!(resolved.report.source, FontSource::Embedded);
        assert!(UnicodeFont::parse(resolved.data).is_ok());
    }

    #[test]
    fn installed_fonts_are_indexed_by_family_and_style_from_their_name_and_head_tables() {
        let root = sandbox("system_index");
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("Sans.ttf"), bundled("LiberationSans-Regular.ttf")).unwrap();
        fs::write(root.join("nested/SansBold.TTF"), bundled("LiberationSans-Bold.ttf")).unwrap();
        fs::write(root.join("broken.ttf"), b"not a font").unwrap();
        let system = InstalledSystemFonts::in_folders(vec![root.clone(), root.join("absent")]);
        let faces = system.faces(" liberation SANS ");
        assert_eq!(faces.iter().map(|face| (face.family.as_str(), face.style)).collect::<Vec<_>>(),
            [("Liberation Sans", FontStyle::Regular), ("Liberation Sans", FontStyle::Bold)]);
        assert!(system.faces("broken").is_empty());
        let resolved = FontResolver::new(BundledFonts::discover(&fonts_root()).unwrap(), system).resolve("Liberation Sans", FontStyle::Bold, &[]).unwrap();
        assert_eq!(resolved.report.source, FontSource::System);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn startup_discovery_verifies_the_fallback_fonts_against_the_manifest_checksums() {
        let fonts = BundledFonts::discover(&fonts_root()).unwrap();
        let expected = FALLBACK_FAMILIES.iter().map(|(family, _)| *family).collect::<Vec<_>>();
        assert_eq!(fonts.fallback_families().collect::<Vec<_>>(), expected);
        for (family, substitutes) in FALLBACK_FAMILIES {
            let entry = fonts.families.iter().find(|entry| entry.family == family).unwrap();
            assert_eq!(entry.names[1..], substitutes.iter().map(|name| normalized(name)).collect::<Vec<_>>()[..], "{family}");
        }
        assert_eq!(fonts.default_family(), "DejaVu Sans");
        assert_eq!(fonts.budget_bytes(), FALLBACK_BUDGET_BYTES);
        assert_eq!(fonts.fallback_bytes(), FALLBACK_BYTES);
        assert!(BundledFonts::discover_installed().is_ok());

        let changed = copy_fonts("changed");
        let mut bytes = fs::read(changed.join("Carlito-Bold.ttf")).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        fs::write(changed.join("Carlito-Bold.ttf"), bytes).unwrap();
        let error = BundledFonts::discover(&changed).unwrap_err();
        assert!(error.message.contains("fallbackCarlitoBold") && error.message.contains("checksum"), "{}", error.message);

        let missing = copy_fonts("missing");
        fs::remove_file(missing.join("LiberationSerif-Italic.ttf")).unwrap();
        let error = BundledFonts::discover(&missing).unwrap_err();
        assert!(error.message.contains("fallbackLiberationSerifItalic"), "{}", error.message);

        let over = copy_fonts("over_budget");
        let manifest = fs::read_to_string(over.join("manifest.json")).unwrap()
            .replace(&format!("\"budgetBytes\": {FALLBACK_BUDGET_BYTES}"), &format!("\"budgetBytes\": {}", FALLBACK_BYTES - 1));
        fs::write(over.join("manifest.json"), manifest).unwrap();
        let error = BundledFonts::discover(&over).unwrap_err();
        assert!(error.message.contains("budget"), "{}", error.message);

        for root in [changed, missing, over] { let _ = fs::remove_dir_all(root); }
    }
}
