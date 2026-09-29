# Toolbox verification map

Read this index, launch a dedicated development instance, and run Doctor before driving a feature. The map follows Toolbox's command center, task workspaces, and Settings panel.

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

- [Command center](./command-center.md) covers search, filters, favorites, recent tools, quick access, and workspace navigation.
- [PDF workflows](./pdf-workflows.md) covers the PDF editor and PDF conversions.
- [Image workflows](./image-workflows.md) covers image editing and media utilities.
- [File security](./file-security.md) covers password removal and file protection.
- [Settings](./settings.md) covers appearance, app information, updates, and privacy preferences.
