# Toolbox Redesign Plan — Phased Execution

**Goal:** Turn Toolbox from a collection of single-purpose tool pages into a
polished, Preview-like desktop application: a command-center home, a merged
PDF editor workspace, a merged photo editor workspace, and native desktop
behavior throughout.

**Visual target:** the polished mockups in `~/workspace/toolbox-redesign/mockups/`
(command center, PDF editor, photo editor) — dark, refined, desktop-native.

**Source material:** the `codex/consolidate-tool-workspaces` branch (tip `d99bdfb`)
holds the working foundation — typed non-destructive scene/edit-plan models,
five new native commands, and real workspace views. `main`/`staging` currently
has only the registry scaffolding; the editor UIs never landed.

## Cross-cutting rules (every phase)

- PRs target `staging`, one PR per phase (split further if a phase gets large).
- Every PR keeps `npm run check:release` and `node docs/check-docs.mjs` green so
  the website never drifts from the app again.
- Codex owns the PostHog branch until PR #335 merges. **Phase 0 waits on #335.**
- No new user-facing tools are added during the redesign; this is strictly a
  UX/architecture overhaul of the existing 33.

## Phase 0 — Integrate the workspace branch onto staging

**Goal:** land the consolidate branch's implementation cleanly: no cruft, no
regressions.

**Work:**
- Merge `codex/consolidate-tool-workspaces` (tip `d99bdfb`) into a fresh branch
  off `staging`. Adjudicate the 4 known overlapping files:
  - 3 workflow files: keep both hunks (branch's `pdftotext.exe` staging +
    staging's Tesseract PATH fix; the product-demo job is already gone).
  - `PDFUnlockView.tsx` (delete/modify): keep the branch's deletion, port
    main's one-line aria-label fix into `SecurityWorkspaceView`.
- Strip agent cruft: `.unlazy/`, `skills/`, `.agents/skills/` (~200 vendored
  files), `decisions.tsv`, `skill.zip`, `skills-lock.json`,
  `CONSOLIDATION_PLAN.md`, `GATES.md` (process docs, not user docs).
- Delete dead `features/pdf-editor/PdfEditor.tsx` after verifying zero imports.
- PostHog sequencing: merge PR #335 to `staging` **first**, then integrate —
  resolve all analytics-file conflicts in favor of #335, keep the branch's side
  only for non-analytics files.
- Run the full gate: `tsc`, `cargo test`, `npm run check:release`,
  `node docs/check-docs.mjs`, production build; fix fallout. Launch the binary
  once as an early smoke test (de-risks Phase 7).

**Acceptance:** clean build, all suites green, no cruft on disk, binary
launches, one PR to `staging`.

**Out of scope:** any visual changes — this phase is mechanical.

## Phase 1 — Design system (build the polish once)

**Goal:** turn the mockups' finish into tokens + primitives so Phases 2–5
inherit it instead of re-skinning later.

**Work:**
- Audit the branch's CSS variables (`--bg/--fg/--surface/--border/--action`)
  and define the refined palette: layered dark surfaces (canvas / panel / card /
  overlay), 1px border colors, soft shadow tokens, the sophisticated blue accent
  with hover/active/disabled/focus-ring states.
- Typography scale (display, heading, body, secondary, mono) with tight
  tracking on headings and a muted secondary text color.
- Component kit: Button (primary/ghost/danger + sizes), IconButton, Slider,
  ColorSwatches, Toggle, SegmentedControl, TextField/NumberField, Select,
  Pill/Badge, Card, Panel/Section, Thumbnail, Tooltip, EmptyState,
  Toolbar + ToolbarButton, StatusBar — each with hover, focus, and disabled
  states.
- Apply to the app shell (MainPage rail + search) as the proving ground.
- Write `DESIGN_SYSTEM.md` (tokens + usage rules) so future work stays
  consistent; add a grep check that no hardcoded colors survive outside tokens.
- Defer full light theme to a later polish pass — mockups are dark-only, and
  doubling Phases 3–5 for light mode now would stall the redesign.

**Acceptance:** shell matches the mockup finish; every primitive has all
interaction states; zero hardcoded colors outside tokens.

## Phase 2 — Command center home

**Goal:** the new home screen (mockup 1).

**Work:**
- Slim app rail (All tools, Favorites, Recent, Settings), ⌘K/Ctrl+K search
  field, breadcrumb + On-device pill.
- Five workspace cards with action chips; chips deep-link into the workspace
  with that tool preselected.
- Favorites + recents in localStorage with empty states.
- Keyboard: ⌘K focuses search, arrows navigate results/cards, Enter opens.

**Acceptance:** all 5 workspaces reachable by click, chip, and search;
favorites persist across restarts; search covers all 33 tools.

## Phase 3 — PDF editor workspace

**Goal:** the Preview-like merged editor (mockup 2).

**Work:**
- Re-skin the branch's `PDFEditorWorkspaceView` onto Phase-1 primitives:
  toolbar (select/text/highlight/shape/signature/watermark/crop), pages sidebar
  (thumbnails, drag-reorder, shift/cmd multi-select, per-page rotate, insert
  blank, delete, page numbers), `SceneCanvas` (pointer gestures, resize handles,
  double-click text edit, Esc cancels), properties panel adapting to selection
  type, status bar (page x of y, zoom, export-preview state).
- Wire the native scene commands: `inspect_pdf_scene`, debounced
  `preview_pdf_scene` re-render, `export_pdf_scene`; export stays disabled
  until the preview verifies.
- Merge/split placement: the branch houses them in the Convert workspace — keep
  them there, but add deep-links from the PDF editor's page-sidebar overflow
  menu so the operations feel one click away.
- Undo/redo (100-deep, grouped strokes), reset, and the non-destructive
  guarantee: original file never touched, export writes alongside with a
  sensible filename.

**Acceptance:** a real PDF round-trips (annotate → sign → watermark →
reorganize → export); Playwright covers each operation; original file
byte-identical after the session.

## Phase 4 — Photo editor workspace

**Goal:** the Lightroom-like merged photo editor (mockup 3).

**Work:**
- Re-skin `ImageEditorWorkspaceView`: tool rail (convert, compress, resize,
  rotate/flip, crop, watermark, tone), center preview with draggable crop
  rectangle + rule-of-thirds grid + watermark position overlay, right inspector
  per tool (tone sliders incl. saturation/exposure, aspect presets, rotation),
  edit-stack list with per-edit undo, "Add edit to plan", single
  `export_image_edit_plan`.
- Switching tools auto-commits the draft edit; live preview via
  `inspect_image_edit_preview`.

**Acceptance:** a real image round-trips with 3+ stacked edits; undo removes
exactly one edit; exported pixels match the preview.

**Note:** Phases 3 and 4 can run in parallel once Phase 1 lands — different
files, different native commands.

## Phase 5 — Convert, Media, Security workspaces

**Goal:** the remaining three workspaces at the same polish bar.

**Work:** re-skin `PDFConversionWorkspaceView` (page-range selection, per-page
previews, OCR lives here), `MediaWorkspaceView` (icons, GIF create/extract,
TIFF, metadata inspect/strip, vision tools), `SecurityWorkspaceView`
(protect/unlock, password fields, batch mode).

**Acceptance:** all 33 registry tools reachable; the three unavailable tools
(OCR PDF, Blur Faces, Remove Background) show clear "unavailable + why" states;
`node docs/check-docs.mjs` still passes in the PR.

## Phase 6 — Desktop-app feel

**Goal:** stop feeling like a website.

**Work:**
- Native menu bar: File (Open, Export, Close window), Edit
  (Undo/Redo/Cut/Copy/Paste), View (zoom controls, theme), Window, Help (About,
  keyboard-shortcut reference).
- Window state persistence (size/position, per workspace where sensible).
- Title bar shows the document (`Toolbox — Document.pdf`); clean drag region.
- Global shortcuts: ⌘K search, ⌘O open, ⌘Z/⇧⌘Z undo/redo, ⌘E export, consistent
  Esc hierarchy.
- File associations + dock/taskbar drag-drop: dropping a PDF opens the PDF
  editor, an image opens the photo editor.
- Single-instance: a second launch focuses the existing window and opens the
  file there.

**Acceptance:** side-by-side feel check against Preview on macOS and
Photos/Paint on Windows; explicit checklist signed off on both.

**Out of scope:** auto-updater UX changes, new tools — pure shell behavior.

## Phase 7 — Verify and release

**Goal:** sign off the branch's never-completed G11 gate and ship.

**Work:** real desktop smoke runs on macOS and Windows across all 5 workspaces;
full Playwright suite green; `npm run check:release`; `staging` → `main` merge;
1.0.14 release; verify installers + signed `tauri-latest.json`; refresh website
screenshots and version copy afterward.

**Acceptance:** release published, updater manifest valid, website links and
screenshots current.

## Risks

- The branch is 12+ days behind `staging`; the merge may surface conflicts
  beyond the 4 known files in shared components.
- G11 (real desktop verification) was never signed off on the branch, so real
  desktop bugs could appear late — the Phase 0 launch smoke exists to catch the
  worst of that early.
- Scope creep on "desktop feel" — Phase 6 is explicitly bounded to shell
  behavior; anything bigger becomes a follow-up.
