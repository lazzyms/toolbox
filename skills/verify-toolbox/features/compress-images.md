# Compress Images

## Sub-features

- Select images through the native Tauri file dialog or drag-and-drop.
- Adjust the visible quality slider from 1 to 100 percent.
- Preserve input format, avoid inflating files, and report per-file results.

## How to get to it (user POV)

Launch Toolbox, then choose `All tools` → `Images` → `Compress Images`.

## Driving it with Tauri desktop UI

Choose a PNG or JPEG fixture, set the visible `Quality` slider from 1 to 100, and click `Compress Images`. The `Lossless (preserve original bytes)` checkbox is also available. Assert a successful result, a `-compressed` output, unchanged original bytes, and an output no larger than the original.

## Gotchas

The frontend sends paths, numeric quality, and the lossless flag to `compress_images`; an unchanged copy is valid when re-encoding would inflate the file.
