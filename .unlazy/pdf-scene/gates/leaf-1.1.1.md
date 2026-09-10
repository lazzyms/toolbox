# Gates: Native scene compositor
OWNS: ToolboxTauri/src-tauri/src/kit/pdf/scene.rs, ToolboxTauri/src-tauri/src/kit/pdf/mod.rs, ToolboxTauri/src-tauri/src/main.rs
Scope: Render and export identical scenes safely, including mixed page trees, origins, crop, rotation, blanks and all object kinds.

- [x] G1: Native scene tests prove composition, geometry, original preservation, collision safety and temporary preview cleanup
  CHECK: cargo test --manifest-path src-tauri/Cargo.toml kit::pdf::scene::tests && echo PDF_SCENE_NATIVE_OK
  EXPECT: PDF_SCENE_NATIVE_OK
  CWD: ToolboxTauri
  EVIDENCE: automatic-evidence=v1; definition-sha256=aac5474c27919a3bcc1d9c1bb89bde9e5efd4e2f66efc605c63ba57396326526; exit=0; EXPECT=matched; output-sha256=a37ebdf5a91dd7215c0ac1b5c1ae8009a1af07c7026ce28b0d54a8792ac3770c; output-bytes=671; shell=/bin/sh; cwd=/Users/mauliksompura/Documents/Projects/Personal/toolbox/ToolboxTauri; path=49172725d4c9/48 entries
