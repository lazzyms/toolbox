# Toolbox verification map

Read this index, launch the verification app, and run Doctor from [verify-toolbox](../SKILL.md) before you drive a feature. The map follows the home screen, the six task workspaces, and Settings.

## Baseline

- Build and open the separate macOS app with the verification config and helper paths in [verify-toolbox](../SKILL.md).
- Drive only the `com.toolbox.desktop.verify` process launched by this run.
- Keep port 1420 and any app window from another run untouched.
- Create fixtures in the run's `scratch/` directory and hash them before processing.
- Use the real file picker and native commands for integrated proof.
- The Playwright specs replace Tauri IPC with stubs. Do not use them to claim that a native operation wrote an output.

## Features

- [Command center](./command-center.md) covers the home rail, search, favorites, and the Tool library.
- [File Security](./file-security.md) covers Lock & Unlock.
- [PDF Editor](./pdf-editor.md) covers the page editor and the whole-file PDF tools.
- [PDF Conversion](./pdf-conversion.md) covers PDF to images, extraction, images to PDF, OCR, and Convert to PDF.
- [Image Editor](./image-editor.md) covers the Edit Image step stack and Convert Image Format.
- [Compress Files](./compress-files.md) covers PDF and image compression.
- [Media](./media.md) covers app icons, GIFs, TIFF pages, and the vision tools.
- [Settings](./settings.md) covers appearance, output location, quick actions, packs, privacy, updates, support, and shortcuts.
