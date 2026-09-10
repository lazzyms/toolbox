# Gates: Direct canvas
OWNS: ToolboxTauri/src/features/pdf-editor/SceneCanvas.tsx, ToolboxTauri/src/features/pdf-editor/scene-canvas.css, ToolboxTauri/tests/ui/scene-canvas.spec.ts
Scope: Pointer-safe direct creation, drag, resize, crop, freehand signatures and selection with accurate transforms.

- [x] G1: Canvas interaction tests exercise creation, drag, resize and rotated cropped coordinate mapping
  CHECK: npm run test:ui -- tests/ui/scene-canvas.spec.ts && echo PDF_SCENE_CANVAS_OK
  EXPECT: PDF_SCENE_CANVAS_OK
  CWD: ToolboxTauri
  EVIDENCE: automatic-evidence=v1; definition-sha256=9dbef0e16b8438d3b307f15b8781b80d4caee9aea8fed9d758435382d80a4d39; exit=0; EXPECT=matched; output-sha256=e41243b1962488566b3d50390f4d355f4ab43ad7f39feb53b6b4fc60beedac77; output-bytes=922; shell=/bin/sh; cwd=/Users/mauliksompura/Documents/Projects/Personal/toolbox/ToolboxTauri; path=49172725d4c9/48 entries
