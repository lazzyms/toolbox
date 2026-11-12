# Convert Image Format

## Sub-features

- Select images through the native Tauri file dialog or drag-and-drop.
- Choose PNG, JPEG, WebP, or HEIC in the target-format buttons.
- Report per-file results such as `Saved as png` and write `{stem}-converted.<ext>` outputs in the target format.

## How to get to it (user POV)

Launch Toolbox, press the `Images` category, then open the `Convert Image Format` card.

## Driving it with Tauri desktop UI

Choose a PNG fixture, press the `📷 JPEG` target-format button (the label includes the emoji), and press `Convert Images`. Assert success, a `.jpg` output with valid JPEG bytes, and unchanged original bytes.

## Gotchas

HEIC support depends on the Rust `heif-rs` build and platform codecs. Verify actual bytes, not only the filename. The Tauri port has no resize flow or menu-bar settings surface.
