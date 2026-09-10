# Gates: PDF editor interaction follow-up

- [x] G1: Direct editor interactions cover inline text, selection highlights, shape variants, two signature modes, and fixed watermark patterns
  CHECK: npm run test:ui -- tests/ui/pdf-editor-followup.spec.ts && echo PDF_EDITOR_FOLLOWUP_OK
  EXPECT: PDF_EDITOR_FOLLOWUP_OK
  CWD: ../../ToolboxTauri
  EVIDENCE: automatic-evidence=v1; definition-sha256=29b649744dde0ebe856a257e1b2fdf647c14c0b01c2742c7f733bffac4a741f3; exit=0; EXPECT=matched; output-sha256=c1e2d4ad7f3f491d68c939b4ff521cacf3e4e9c34c911e0b2ea39cb3f4c90074; output-bytes=1826; shell=/bin/sh; cwd=/Users/mauliksompura/Documents/Projects/Personal/toolbox/ToolboxTauri; path=49172725d4c9/48 entries

- [x] G2: Shared scene payload and native compositor export all edits in one new PDF
  CHECK: npm run build && cargo test --manifest-path src-tauri/Cargo.toml && echo PDF_EDITOR_FOLLOWUP_OK
  EXPECT: PDF_EDITOR_FOLLOWUP_OK
  CWD: ../../ToolboxTauri
  EVIDENCE: automatic-evidence=v1; definition-sha256=b37c6d8a18509e1208d5778e39bd31cfebe781f6ed8f4e9895b72ad5bcec9cf4; exit=0; EXPECT=matched; output-sha256=33f39d733613371a9ec3d18ed02b33fba4796c60fe6ec3eab10b3e9899b8bc28; output-bytes=8075; shell=/bin/sh; cwd=/Users/mauliksompura/Documents/Projects/Personal/toolbox/ToolboxTauri; path=49172725d4c9/48 entries

- [x] G3: The consolidated editor remains above-fold, release-safe, and preserves the on-device/source separation
  CHECK: npm run test:ui && npm run check:release && node scripts/check-workspace-consolidation.mjs && echo PDF_EDITOR_FOLLOWUP_OK
  EXPECT: PDF_EDITOR_FOLLOWUP_OK
  CWD: ../../ToolboxTauri
  EVIDENCE: automatic-evidence=v1; definition-sha256=c080f9d7b116129e7a2c6d076c171d08e2b6a244f9096495e82759fbc3212abf; exit=0; EXPECT=matched; output-sha256=6218f9a9a17480818b793ff939de5f0c00d2adde5f96a7cdee6180cf8d673387; output-bytes=10661; shell=/bin/sh; cwd=/Users/mauliksompura/Documents/Projects/Personal/toolbox/ToolboxTauri; path=49172725d4c9/48 entries
