# Convert Image Format

## Sub-features

- Select images through the native Tauri file dialog or drag-and-drop.
- Choose PNG, JPEG, WebP, or HEIC in the target-format buttons.
- Report per-file results and write `-converted` outputs in the target format.

## How to get to it (user POV)

Launch Toolbox, then choose `Convert` in the Images section of the sidebar.

## Driving it with Tauri desktop UI

Choose a PNG fixture, click the `JPEG` target-format button, and click `Convert Images`. Assert success, a `.jpg` output with valid JPEG bytes, and unchanged original bytes.

## Gotchas

HEIC support depends on the Rust `heif-rs` build and platform codecs. Verify actual bytes, not only the filename. The Tauri port has no resize flow or menu-bar settings surface.
