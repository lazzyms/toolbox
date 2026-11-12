---
name: verify-toolbox
description: "Verify Toolbox's cross-platform Tauri desktop app through its React/Vite window, native file dialog, drag-and-drop path, Rust IPC commands, and filesystem outputs. Use after Tauri UI, Rust kit, packaging, or updater changes, or when a smoke proof is needed."
---

# Verify Toolbox Tauri

Toolbox's current product surface is the cross-platform Tauri desktop app under `ToolboxTauri/`. It has a React/Vite frontend and Rust backend commands. The legacy SwiftUI macOS app is out of scope for this skill.

## Launch

From the repository root, install frontend dependencies (again after any dependency change) and build the offline vision adapter once:

```bash
cd ToolboxTauri
npm ci
cargo build --release --manifest-path src-tauri/Cargo.toml --features vision-adapter --bin toolbox-vision-adapter
```

Start the development app with the `toolbox` binary named explicitly. The crate has two binaries and no `default-run`, so a plain `npm run tauri dev` fails. Point analytics at a dead local port so a verification run never sends telemetry, and point the helper overrides at local tools so every feature can produce output:

```bash
cd ToolboxTauri
M="$PWD/src-tauri/resources/vision/models"
A="$PWD/src-tauri/target/release/toolbox-vision-adapter"
POSTHOG_PROJECT_TOKEN=verification-disabled POSTHOG_HOST=http://127.0.0.1:9 \
TOOLBOX_QPDF_PATH="$(command -v qpdf)" \
TOOLBOX_PDFTOPPM_PATH="$(command -v pdftoppm)" \
TOOLBOX_TESSERACT_PATH="$(command -v tesseract)" \
TOOLBOX_TESSDATA_PATH="$(brew --prefix)/share/tessdata/eng.traineddata" \
TOOLBOX_FACE_BLUR_PATH="$A" TOOLBOX_FACE_BLUR_MODEL_PATH="$M/face_detection_yunet_2023mar.onnx" \
TOOLBOX_BACKGROUND_REMOVAL_PATH="$A" TOOLBOX_BACKGROUND_REMOVAL_MODEL_PATH="$M/u2netp.onnx" \
npm run tauri -- dev -- --bin toolbox
```

The Vite dev server listens on `http://localhost:1420`; Tauri opens a desktop window titled `Toolbox`. On macOS, install the helpers with `brew install qpdf poppler tesseract`. On Windows, install qpdf with `choco install qpdf -y`; release builds bundle it beside the executable through `scripts/bundle-qpdf.ps1`. The dev log may show a PostHog or OpenTelemetry batch-processor panic with the inert analytics settings; the window still works.

For automated browser UI coverage of every registered feature, run `npm run test:ui` from `ToolboxTauri/`. The suite starts Vite, selects each of the 32 registry entries in a fresh page, supplies the checked-in app-icon fixture through the dialog bridge, exercises the feature action, and asserts the corresponding Tauri command plus a successful result. This verifies navigation, pane controls, fixture handling, and command dispatch; native processing remains covered by the live drive and native tests below.

For native E2E coverage of all 32 registered commands, run `npm run test:native:e2e` from `ToolboxTauri/`. The test creates valid PDF, PNG, animated GIF, and TIFF fixtures under a unique temporary root, gives every command its own output directory, verifies real output files or explicit unavailable vision adapters, and proves the source fixtures remain unchanged.

For a packaged smoke run, use `npm run tauri build` and launch the unsigned app produced under `ToolboxTauri/src-tauri/target/release/bundle/`. The build may omit updater artifacts when `TAURI_SIGNING_PRIVATE_KEY` is absent. Never drive a user's installed Toolbox while a verification run is active. Keep one dev instance per run; the dev server port and Tauri window are shared resources.

Teardown: close the Tauri window, stop the `npm run tauri` process you started, and remove only the run's scratch directory.

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

On a live run, also confirm `http://localhost:1420` answers and that the desktop window is titled `Toolbox`. `cargo test` exercises the Rust kit and the IPC-facing command shapes; it is not a substitute for a real WebView drive.

## Drive

Use the Tauri desktop window through the platform UI harness (macOS Accessibility/CUA or Windows UI Automation). Prefer visible labels and text; use coordinates only where noted below. On macOS, read the window with an explicit accessibility timeout of at least 5 seconds; the WebView tree is large and the default can time out.

1. Confirm the window title is `Toolbox`. With nothing chosen, the detail pane says `No tool selected.`
2. Open a tool: press a category button (`Documents`, `PDF`, `Images`) or `All tools`, then press the card button named `Open <full card title>`, for example `Open Protect PDF` or `Open Extract GIF Frames`. The detail pane then announces `<full card title> selected.` Full card titles are listed in the [feature map](features/README.md).
3. Press `Choose files to process` (the `Drag & Drop files here` area) and pick files in the native dialog. Assert `N files selected` and each filename appear.
4. Exercise the feature's visible controls, then press its run button. The run button usually repeats the sidebar short title (`Watermark`, `Split`, `Crop`), so the same label also exists in the sidebar: press the last match in the window. Feature files name the run buttons that differ.
5. Wait for `1 files completed` (or `N files completed` / `files failed`) and assert the result row text and output filenames. Inspect the filesystem output and prove the input bytes remain unchanged.

macOS file dialog recipes, most robust first:

- The dialog opens as an `AXSheet` (`id=open-panel`) inside the Toolbox window's accessibility tree. When it already shows the fixture folder (it remembers the last folder), find the row `AXCell` whose text field equals the filename and perform its `AXOpen` action. The action may report error `-25205` even though it worked; assert `1 files selected` instead of trusting the return code.
- Otherwise use the Go-to-folder path: with Toolbox frontmost, send `cmd+shift+g` in foreground mode to the topmost on-screen `Open and Save Panel Service` window, type the exact absolute fixture path, press Return, wait about 1.5 seconds, and press Return again (or press the sheet's `Open` button, `id=OKButton`). Keystrokes are lost if another app is frontmost. Setting the value of `PathTextField` directly works; its accessibility `confirm` action does nothing.
- Filter Panel Service windows on `is_on_screen`; stale off-screen copies linger after earlier dialogs.
- Typing tools may report the text as unverified even when it landed. Check the field or the selection count instead of retyping, which duplicates text.
- A stuck sheet is cleared by pressing its `Cancel` button (`id=CancelButton`).
- To add a second file, click the dropzone icon with a foreground pixel click. An accessibility press on the dropzone while files are listed lands on its nested `Clear all` button and empties the list.
- If a dialog asks to replace an existing file, choose `Cancel`; never overwrite a fixture.

Other control notes: page-selection toggles, mode toggles, `Loop animation`, and the Settings theme options are exposed as checkboxes. Crop PDF needs a foreground mouse drag across the rendered page; there is no field-based rectangle entry. Never press `Show in Finder`, `Open file`, or `Check for updates` during verification.

The frontend invokes every command listed in the feature map through Tauri IPC. Do not call commands directly for a live UI proof. PDF rendering uses `pdftoppm`; PDF protection, unlocking, merging, splitting, and compression use qpdf. Remove Password uses the native Rust Office adapter for Word, Excel, and PowerPoint files and does not require an Office runtime. OCR uses tesseract; face blur and background removal use the `toolbox-vision-adapter` binary with the ONNX models in `ToolboxTauri/src-tauri/resources/vision/models/`. An absent adapter is an expected, explicit unsupported result, not a pass. Use the environment overrides from Launch rather than `docs/tauri-vision-engines.md` for the adapter wiring.

## Evidence

Store proof artifacts under a run-specific `.verification/tauri-<timestamp>/evidence/` directory. Capture a screenshot showing the selected tool, fixture filename, visible option, and run button; a screenshot showing the resulting per-file row; `npm run build` and `cargo test` transcripts; and hashes or byte comparisons proving originals were unchanged, plus output extension, suffix, and encryption/format checks.

Exercise the real user path through the Tauri window, file dialog, and run button. Verify visible results and filesystem side effects together. Use disposable local fixtures and passwords in the run's own fixture folder; do not upload files or use production secrets. Before closing any file dialog, confirm it is pointed at the fixture folder so no copy lands in a personal folder. For automation, run `npm run check:release` and `cargo test --manifest-path ToolboxTauri/src-tauri/Cargo.toml`; these prove contracts and native behavior but do not replace a live WebView drive.

## Cleanup

Stop only the Tauri dev process and Vite process started by this run; never kill by a broad process name. Close the verification window, remove its scratch inputs/outputs, and preserve `evidence/`. If the dev server port is already owned by another process, stop and report the owner instead of reusing it.

## Helpers

The canonical commands are `npm run tauri -- dev -- --bin toolbox`, `npm run build`, and `cargo test` from the directories above. If repeated verification is needed, add a helper only inside `skills/verify-toolbox/` and document its exact invocation here.

Run `node skills/verify-toolbox/scripts/run-evidence.mjs MANIFEST EVIDENCE_DIR ROOT` to validate a manifest-driven evidence set. The command writes only the run-specific evidence directory and uses relative paths in its report. Its self-test runs with `node --test skills/verify-toolbox/scripts/run-evidence.test.mjs`.

## Feature map

See [`features/README.md`](features/README.md) for the maintained Tauri user-facing feature map. Keep it aligned with `ToolboxTauri/src/registry/index.ts`, `ToolboxTauri/src/views/`, `ToolboxTauri/src-tauri/src/main.rs`, and the vision adapter under `ToolboxTauri/src-tauri/src/`.
