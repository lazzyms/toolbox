---
name: verify-toolbox
description: "Verify Toolbox's cross-platform Tauri desktop app through its React/Vite window, native file dialog, drag-and-drop path, Rust IPC commands, and filesystem outputs. Use after Tauri UI, Rust kit, packaging, or updater changes, or when a smoke proof is needed."
---

# Verify Toolbox Tauri

Toolbox's current product surface is the cross-platform Tauri desktop app under `ToolboxTauri/`. It has a React/Vite frontend and Rust backend commands. The legacy SwiftUI macOS app is out of scope for this skill.

## Launch

From the repository root, install frontend dependencies once and start the Tauri development app:

```bash
cd ToolboxTauri
npm ci
npm run tauri dev
```

The Vite dev server listens on `http://localhost:1420`; Tauri opens a desktop window titled `Toolbox`. On macOS, install qpdf with `brew install qpdf` if PDF protection or unlocking is being exercised. On Windows, install qpdf with `choco install qpdf -y`; release builds bundle it beside the executable through `scripts/bundle-qpdf.ps1`.

For automated browser UI coverage of every registered feature, run `npm run test:ui` from `ToolboxTauri/`. The test starts Vite, selects each registry entry, and asserts the matching detail heading and live status. This verifies navigation and pane reachability; feature processing remains covered by the live drive and native tests below.

For a packaged smoke run, use `npm run tauri build` and launch the unsigned app produced under `ToolboxTauri/src-tauri/target/release/bundle/`. The build may omit updater artifacts when `TAURI_SIGNING_PRIVATE_KEY` is absent. Never drive a user's installed Toolbox while a verification run is active. Keep one dev instance per run; the dev server port and Tauri window are shared resources.

Teardown: close the Tauri window, stop the `npm run tauri dev` process you started, and remove only the run's scratch directory.

## Doctor

Run these read-only checks before driving:

```bash
cd ToolboxTauri
npm run build
cd src-tauri
cargo test
cd ..
npm run check:release
```

On a live run, also confirm `http://localhost:1420` answers and that the focused desktop window is titled `Toolbox`. `cargo test` exercises the Rust kit and the IPC-facing command shapes; it is not a substitute for a real WebView drive.

## Drive

Use the Tauri desktop window through the platform UI harness (macOS Accessibility/CUA or Windows UI Automation). Prefer visible labels and text; do not use coordinates when a text or role selector is available.

1. Confirm the window title is `Toolbox` and the empty state says `Ready to process`.
2. Select one of the four sidebar buttons: `Unlock PDF`, `Protect PDF`, `Compress`, or `Convert`.
3. Click the `Drag & Drop files here` area (or drop real fixture paths) and use the native file dialog to select files. Assert the selected count and filename appear.
4. Exercise the feature's visible control: `PDF Password` + `Unlock PDF`, `Encryption Password` + `Protect PDF`, `Quality` + `Compress Images`, or a target format such as `JPEG` + `Convert Images`.
5. Wait for `Processing batch…` to disappear and assert the result row is green and contains the expected detail. Inspect the filesystem output and prove the input bytes remain unchanged.

The frontend invokes every command listed in the feature map through Tauri IPC. Do not call commands directly for a live UI proof. PDF rendering uses `pdftoppm`; PDF protection, merging, and splitting use qpdf. OCR, face blur, and background removal use offline adapters documented in `docs/tauri-vision-engines.md`; an absent adapter is an expected, explicit unsupported result, not a pass. The surface includes image geometry/effects/formats, PDF conversion/editor/selection tools, accessibility semantics, and release checks.

## Evidence

Store proof artifacts under a run-specific `.verification/tauri-<timestamp>/evidence/` directory. Capture a screenshot showing the selected tool, fixture filename, visible option, and run button; a screenshot showing the resulting per-file row; `npm run build` and `cargo test` transcripts; and hashes or byte comparisons proving originals were unchanged, plus output extension, suffix, and encryption/format checks.

Exercise the real user path through the Tauri window, file dialog, and run button. Verify visible results and filesystem side effects together. Use disposable local fixtures and passwords; do not upload files or use production secrets. For automation, run `npm run check:release` and `cargo test --manifest-path ToolboxTauri/src-tauri/Cargo.toml`; these prove contracts and native behavior but do not replace a live WebView drive.

## Cleanup

Stop only the Tauri dev process and Vite process started by this run; never kill by a broad process name. Close the verification window, remove its scratch inputs/outputs, and preserve `evidence/`. If the dev server port is already owned by another process, stop and report the owner instead of reusing it.

## Helpers

The canonical commands are `npm run tauri dev`, `npm run build`, and `cargo test` from the directories above. If repeated verification is needed, add a helper only inside `skills/verify-toolbox/` and document its exact invocation here.

Run `node skills/verify-toolbox/scripts/run-evidence.mjs MANIFEST EVIDENCE_DIR ROOT` to validate a manifest-driven evidence set. The command writes only the run-specific evidence directory and uses relative paths in its report.

## Feature map

See [`features/README.md`](features/README.md) for the maintained Tauri user-facing feature map. Keep it aligned with `ToolboxTauri/src/registry/index.ts`, `ToolboxTauri/src/views/`, `ToolboxTauri/src-tauri/src/main.rs`, and the native adapters documented in `docs/tauri-vision-engines.md`.
