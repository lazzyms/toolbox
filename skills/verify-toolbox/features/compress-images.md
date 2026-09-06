# Compress Images

## Sub-features

- Select images through the native Tauri file dialog or drag-and-drop.
- Adjust the visible quality slider from 1 to 100 percent.
- Preserve input format, avoid inflating files, and report per-file results.

## How to get to it (user POV)

Launch Toolbox, then choose `Compress` in the Images section of the sidebar.

## Driving it with Tauri desktop UI

Choose a PNG or JPEG fixture, set the `Quality` slider, and click `Compress Images`. Assert a successful result, a `-compressed` output, unchanged original bytes, and an output no larger than the original.

## Gotchas

The frontend sends paths and numeric quality to `compress_images`. It does not expose the legacy Swift lossless/lossy picker or metadata toggle; an unchanged copy is valid when re-encoding would inflate the file.
