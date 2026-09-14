# Gates: Workspace
OWNS: ToolboxTauri/src/features/pdf-editor/scene.ts, ToolboxTauri/src/views/PDFEditorWorkspaceView.tsx, ToolboxTauri/src/index.css, ToolboxTauri/src/views/MainPage.tsx, ToolboxTauri/tests/ui/pdf-scene.spec.ts, ToolboxTauri/tests/ui/feature-navigation.spec.ts, ToolboxTauri/scripts/check-workspace-consolidation.mjs
Scope: One document and history across every tool, thumbnail edits, contextual inline controls and above-fold preview.

- [x] G1: Scene tests prove multiple object kinds, page changes, undo redo reset and single export
  CHECK: npm run test:ui -- tests/ui/pdf-scene.spec.ts && echo PDF_SCENE_WORKSPACE_OK
  EXPECT: PDF_SCENE_WORKSPACE_OK
  CWD: ToolboxTauri
  EVIDENCE: automatic-evidence=v1; definition-sha256=c7e2ff48fe74ac93ec66241f36bb66c7c5bc141e605e33aca770847347b9996f; exit=0; EXPECT=matched; output-sha256=01c8592353f32f9280e7bffd237849f02efca775d2f536ceaa99b26405728561; output-bytes=1896; shell=/bin/sh; cwd=/Users/mauliksompura/Documents/Projects/Personal/toolbox/ToolboxTauri; path=49172725d4c9/48 entries

- [x] G2: Above-fold page and toolbar measurements pass in desktop light and dark layouts
  CHECK: npm run test:ui -- tests/ui/pdf-scene.spec.ts --grep "above the fold" && echo PDF_SCENE_LAYOUT_OK
  EXPECT: PDF_SCENE_LAYOUT_OK
  CWD: ToolboxTauri
  EVIDENCE: automatic-evidence=v1; definition-sha256=4aa3ac5a370e281ec7d0188f0cc2bb1d89f6ce2d8687fffffd70e420694914e6; exit=0; EXPECT=matched; output-sha256=25fb914ce89adab1f3676eb15989b0a56195cfcc340a5321190878e6a53e8852; output-bytes=916; shell=/bin/sh; cwd=/Users/mauliksompura/Documents/Projects/Personal/toolbox/ToolboxTauri; path=49172725d4c9/48 entries
