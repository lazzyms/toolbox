# Toolbox Tauri feature map

The primary surface is the cross-platform Tauri desktop window. Every registry entry below has a dedicated verification recipe. All 32 entries are registered as implemented and invoke a typed Rust command through Tauri IPC. The vision tools (OCR, Blur Faces, Remove Background) show their chooser and report a per-file unavailable failure when their offline adapter is absent.

Entry point `PDF → X` means: click the `PDF` category filter, then the card button `Open <full title>`. The run button uses the short title shown here.

| Feature | Entry point | Automated proof |
| --- | --- | --- |
| [Remove Password](unlock-pdf.md) | Documents → Remove Password | PDF and Office password removal |
| [Page numbers](pdf-page-numbers.md) | PDF → Page Numbers | page geometry and overlay |
| [Merge PDF](pdf-merge.md) | PDF → Merge | qpdf output page count |
| [Watermark PDF](pdf-watermark.md) | PDF → Watermark | overlay output and original preservation |
| [Crop PDF](pdf-crop.md) | PDF → Crop | MediaBox and selected scope |
| [Edit PDF](pdf-edit.md) | PDF → Edit | typed annotation output and original preservation |
| [Protect PDF](protect-pdf.md) | PDF → Protect | encrypted output round-trip |
| [Images to PDF](images-to-pdf.md) | PDF → Images to PDF | page count and dimensions |
| [PDF to Images](pdf-to-images.md) | PDF → PDF to Images | rendered image output |
| [PDF to Text](pdf-to-text.md) | PDF → PDF to Text | extracted selectable text |
| [Split PDF](pdf-split.md) | PDF → Split | per-page, range, or chunk outputs |
| [Extract PDF images](pdf-image-extract.md) | PDF → Extract Images | original JPEG bytes |
| [Sign PDF](pdf-sign.md) | PDF → Sign | visible overlay output |
| [OCR PDF](pdf-ocr.md) | PDF → OCR | adapter success or explicit unsupported |
| [Remove pages](pdf-remove-pages.md) | PDF → Remove Pages | selected pages absent |
| [Extract pages](pdf-extract-pages.md) | PDF → Extract Pages | selected pages retained |
| [Organize PDF](pdf-organize.md) | PDF → Organize | order/rotation/delete plan |
| [Compress PDF](pdf-compress.md) | PDF → Compress | valid smaller or preserved output |
| [Convert image format](convert-image-format.md) | Images → Convert | valid target bytes |
| [Compress images](compress-images.md) | Images → Compress | no inflation and original preservation |
| [Resize images](resize-images.md) | Images → Resize | expected dimensions |
| [Rotate images](rotate-images.md) | Images → Rotate | expected orientation |
| [Crop images](crop-images.md) | Images → Crop | expected pixel rectangle |
| [App icons](image-icons.md) | Images → Icons | selected preset outputs |
| [Create GIF](gif-create.md) | Images → GIF Maker | animated frame count |
| [Extract GIF frames](gif-extract.md) | Images → Frames | frame outputs |
| [Image watermark](image-watermark.md) | Images → Watermark | changed output and original preservation |
| [Image metadata](image-metadata.md) | Images → Metadata | output metadata policy |
| [Image tone](image-tone.md) | Images → Tone | changed pixel values |
| [TIFF pages](tiff-pages.md) | Images → TIFF Pages | TIFF output validity |
| [Blur faces](image-blur-faces.md) | Images → Blur Faces | adapter success or explicit unsupported |
| [Remove background](image-remove-bg.md) | Images → Cutout | adapter success or explicit unsupported |

## Surfaces outside the registry

- **Search.** The `Search tools` field (Cmd/Ctrl+K) filters cards. An empty match shows `No matching tools`.
- **Favorites and Recent.** Card star buttons `Add <title> to favorites` and the tool header `☆ Save` / `★ Saved` store favorites in localStorage `toolbox-favorites`. Opened tools go to `toolbox-recent` and appear under `Pick up where you left off`, the `Recent` filter, and `Quick access`.
- **Settings.** The sidebar `Settings` button opens a dialog with Appearance (Dark or Light), Quick actions (six checkboxes saved through `set_quick_actions` to `quick-actions.json` in the app config directory), App info, Support Toolbox, and Updates. Verify that a toggled checkbox survives reopening the dialog.
- **Finder and File Explorer quick actions.** `toolbox --quick-action <id> <paths...>` runs headless with ids `compress-pdf`, `pdf-to-images`, `compress-image`, `convert-to-png`, `convert-to-jpg`, and `remove-background`, and reports through an OS notification. Release builds register the enabled actions at startup (macOS `~/Library/Services/*.workflow`, Windows `HKCU` `SystemFileAssociations` verbs). Debug builds skip registration, so drive the flag directly against the debug binary. `cargo test` covers the registration plans.
- **Result actions.** Each output row has `Open file` and `Show in Finder` (`Open file location` on Windows).

Cross-cutting coverage includes native file selection and drop paths, per-file result isolation, collision-safe output naming, keyboard operation, Windows path display, and `npm run check:release`.
