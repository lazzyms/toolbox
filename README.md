# Toolbox

Toolbox is a cross-platform desktop app for private, local-first PDF, document, and image
utilities. Files are processed on-device and are never uploaded.

The Tauri app on `tauri-port` is the primary Toolbox app and the repository's
default branch. The legacy Swift macOS app remains on `main` with its own release
workflow. The website in [`docs/`](docs/) is kept independently.

## Install the cross-platform release

Download the latest installer from the
[Tauri releases](https://github.com/lazzyms/toolbox/releases?q=tauri-v).

### macOS

1. Open the downloaded `.dmg` file.
2. Drag **Toolbox** to **Applications**.
3. In Applications, Control-click **Toolbox**, choose **Open**, and choose
   **Open** again if macOS asks for confirmation.

### Windows

1. Run the downloaded `.exe` installer.
2. If Windows SmartScreen appears, choose **More info**, then **Run anyway**.

The installers bundle the native PDF helpers they need. Remove Password uses a
native Rust adapter for Word, Excel, and PowerPoint files, with no Office runtime dependency. Toolbox checks for signed
updates shortly after launch and asks before installing one.

## Features

Toolbox currently provides 32 utilities covering PDF and Office password handling, page
editing, merging, splitting, extraction, conversion, OCR, signing, watermarking,
numbering, compression, image conversion, compression, resizing, rotation,
cropping, watermarking, metadata removal, tone adjustment, icon generation, GIF
creation and extraction, TIFF processing, face blurring, and background removal.

Every registered feature has UI automation and native fixture coverage. The app
keeps original files unchanged, avoids overwriting outputs, and isolates failures
per input file.

## Development

Requires Node.js 22+, Rust stable, and the platform toolchain for the target OS.

```bash
cd ToolboxTauri
npm ci
npm run test:ui
npm run test:analytics
npm run test:native:e2e
npm run check:release
npm run build
```

To run the frontend locally:

```bash
cd ToolboxTauri
npm run dev
```

To build a native bundle:

```bash
cd ToolboxTauri
npm run tauri build
```

## Releases

Pushing to `tauri-port` runs [the Tauri release workflow](.github/workflows/tauri-release.yml),
which builds macOS arm64 and Windows arm64/x86_64 artifacts, signs updater metadata,
and publishes a `tauri-v*` GitHub release.

The Swift release workflow and Swift source are intentionally retained on `main`
for the legacy macOS release line. Changes to that line do not belong in this
branch. The `docs/` website and its appcast are also intentionally left unchanged.

## Website

`docs/` is a static site with no build step. Keep website changes isolated from
the application code and do not add trackers or third-party embeds.

## Contributing

Keep processing behavior in `ToolboxTauri/src-tauri/`, UI behavior in
`ToolboxTauri/src/`, and tests in `ToolboxTauri/tests/`. Add or update automated
coverage with every behavior change. Use short imperative commit messages and
open pull requests against `tauri-port`.

Toolbox is released under the [MIT License](LICENSE).
