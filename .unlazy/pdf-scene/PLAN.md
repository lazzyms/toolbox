# Plan: Direct PDF editing
Scope: pdf-scene
Depth: tree 3
Mode: orchestrated

## Contract
Interfaces: ToolboxTauri/src/features/pdf-editor/scene.ts is the shared schema. Coordinates are top-left points in the visible source page, before user rotation and crop. Scene pages define final ordering; null sourceIndex means blank. All objects remain editable and live in one immutable scene. Native inspect_pdf_scene accepts {request:{path}} and returns existing PdfDocument metadata normalized to visible source coordinates. preview_pdf_scene accepts {request:{path,scene,pageIndex}} and returns ScenePreview. export_pdf_scene accepts {request:{paths,scene,outputLocation:"alongsideInput"}} and returns JobOutcome[]. Preview and export use the same native compositor. No original modification or network access.
Host launch mode: Codex native subagents; independent native and canvas leaves launch together, driver implements integration after shared schema.
Toolchain: existing Node/npm/TypeScript/Playwright and Rust/Cargo; cwd repository root unless CWD specified.
Manual review: driver verifies synthetic PDF in Preview and exact dev Tauri process. No private documents used.
Wave policy: native and canvas in wave 1; driver workspace integration overlaps only disjoint owned files. No commits or publication in this scope.

## Current contract inventory
Contract revision: 1
| ID | Required outcome or constraint | Owner | Observing gate or manual review | Disposition | Revision |
|---|---|---|---|---|---|
| C1 | Directly create, drag, resize and select text, highlights, shapes, signatures and watermarks together | 1.1.2 | leaf-1.1.2:G1; node-1.1:N2 | ACTIVE | 1 |
| C2 | Native preview and one export reflect the same edited result | 1.1.1 | leaf-1.1.1:G1; node-1.1:N2 | ACTIVE | 1 |
| C3 | Crop plus thumbnail insertion, deletion, rotation and reorder remain editable | 1.2.1 | leaf-1.2.1:G1 | ACTIVE | 1 |
| C4 | Unified undo redo reset across tools and page mutations | 1.2.1 | leaf-1.2.1:G1 | ACTIVE | 1 |
| C5 | Preview above fold, small title, inline tools, Preview reference | 1.2.1 | leaf-1.2.1:G2; GATES:G3 | ACTIVE | 1 |
| C6 | Local processing, original byte preservation, collision-safe exports, preview cleanup | 1.1.1 | leaf-1.1.1:G1 | ACTIVE | 1 |
| C7 | Keep atomic routes and unrelated work, regressions and native verification | 1.2 | node-1.2:N2; GATES:G2 | ACTIVE | 1 |

## Tree
- 1 PDF editor .. GATES.md
  - 1.1 Editing engine .. gates/node-1.1.md
    - 1.1.1 Native scene compositor .. gates/leaf-1.1.1.md
    - 1.1.2 Direct canvas .. gates/leaf-1.1.2.md
  - 1.2 Workspace .. gates/node-1.2.md
    - 1.2.1 Session integration and layout .. gates/leaf-1.2.1.md

## Leaf dispatch table
| Leaf | Owns | Needs | Tier | Planned wave | State |
|---|---|---|---|---|---|
| 1.1.1 | ToolboxTauri/src-tauri/src/kit/pdf/scene.rs, ToolboxTauri/src-tauri/src/kit/pdf/mod.rs, ToolboxTauri/src-tauri/src/main.rs | - | judgment | 1 | VERIFIED |
| 1.1.2 | ToolboxTauri/src/features/pdf-editor/SceneCanvas.tsx, ToolboxTauri/src/features/pdf-editor/scene-canvas.css, ToolboxTauri/tests/ui/scene-canvas.spec.ts | - | judgment | 1 | VERIFIED |
| 1.2.1 | ToolboxTauri/src/features/pdf-editor/scene.ts, ToolboxTauri/src/views/PDFEditorWorkspaceView.tsx, ToolboxTauri/src/index.css, ToolboxTauri/src/views/MainPage.tsx, ToolboxTauri/tests/ui/pdf-scene.spec.ts, ToolboxTauri/tests/ui/feature-navigation.spec.ts, ToolboxTauri/scripts/check-workspace-consolidation.mjs | - | judgment | 1 | VERIFIED |
