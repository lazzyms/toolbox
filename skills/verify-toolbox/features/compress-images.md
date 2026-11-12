# Compress Images

## Sub-features

- Select images through the native Tauri file dialog or drag-and-drop.
- Adjust the visible `Quality` slider from 1 to 100 percent (default 80), or tick `Lossless (preserve original bytes)`.
- Preserve input format, avoid inflating files, and report per-file results.

## How to get to it (user POV)

Launch Toolbox, press the `Images` category, then open the `Compress Images` card.

## Driving it with Tauri desktop UI

Choose a PNG or JPEG fixture, set the `Quality` slider, and press `Compress Images`. Assert a successful result, a `{stem}-compressed.jpg` output for a JPEG input, unchanged original bytes, and an output no larger than the original.

## Gotchas

The frontend sends paths, numeric quality, and the lossless flag to `compress_images`. It has no metadata toggle; an unchanged copy is valid when re-encoding would inflate the file.
