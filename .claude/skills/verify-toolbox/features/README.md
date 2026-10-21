# Toolbox verification map

Read this index, launch the verification bundle, and run `helpers/doctor.sh` before driving a feature. The map follows Toolbox's home view, its sidebar Views, the tool workspaces, and Settings.

## Baseline

- Build and run the separate macOS app with the verification config in [verify-toolbox](../SKILL.md).
- Drive the native window whose webview URL is `tauri://localhost`.
- Keep port 1420 and any app window from another run untouched.
- Drive only the `com.toolbox.desktop.verify` window launched by this run.
- Copy input files into the run's `scratch/` directory before processing.
- Use the real file picker and native commands for integrated proof.
- The Tauri overlay gives the macOS verification bundle its own app identifier. This map does not cover Windows or Linux launch paths.
- The Playwright specs replace Tauri IPC with stubs. Do not use them to claim that a native operation wrote an output.

## Features

- [Command center](./command-center.md) covers the home sidebar, Views, tool search, favorites, and opening a workspace.
- [PDF workflows](./pdf-workflows.md) covers the PDF Editor and PDF Conversion views.
- [Image workflows](./image-workflows.md) covers the Image Editor and Media views.
- [File security](./file-security.md) covers Lock & Unlock. Proven end to end on 2026-10-20 with `lock-unlock` in Unlock mode.
- [Settings](./settings.md) covers appearance, app information, updates, and privacy preferences.
