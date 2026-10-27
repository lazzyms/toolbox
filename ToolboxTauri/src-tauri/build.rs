fn main() {
    println!("cargo:rerun-if-changed=capabilities");
    // The updater permission exists only when the updater plugin is compiled in.
    let capabilities = if std::env::var_os("CARGO_FEATURE_UPDATER").is_some() {
        "./capabilities/*.json"
    } else {
        "./capabilities/default.json"
    };
    tauri_build::try_build(tauri_build::Attributes::new().capabilities_path_pattern(capabilities))
        .expect("failed to run the Tauri build script");
}
