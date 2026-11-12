# Image Editor

The Image Editor workspace edits many images with a stack of steps and converts image formats.

## Sub-features

- `edit-image` runs a step stack (Crop, Resize, Rotate and Flip, Colour and Tone, Watermark, Remove Image Metadata) into `-edited` copies.
- `convert-image-format` writes `-converted` copies as JPEG, PNG, WebP, HEIC, or TIFF.

## How to get to it (user POV)

- Choose `Image Editor` in the home rail, or `Open Edit Image` or `Open Convert Image Format` in the Tool library.
- Switch tools with the toolbar items `Edit Image` and `Convert Image Format`.

## Driving it with CUA desktop control

Preconditions:

- A JPEG fixture copy is in `scratch/`.

- **Step stack.** In `Edit Image`, add steps with `Add Crop step`, `Add Rotate and Flip step`, `Add Colour and Tone step`, `Add Watermark step`, and `Add Remove Image Metadata step`. Each step is listed as `Step N of M, <step>` with a `Remove <step> step` button. `Undo, ⌘Z`, `Redo, ⇧⌘Z`, and `Reset stack` act on the whole stack. The tab radios `Resize`, `Rotate`, `Crop`, `Watermark`, `Remove Metadata`, and `Adjust` show each step's options.
- **Rotate.** Add a Rotate and Flip step, choose `90 degrees`, and choose `Run on 1 image`. Expect `Done. 1 rotated` and `<name>-edited.jpg`.
- **Resize.** With a Resize step, choose `Longest side`, type `128` in `Longest side, pixels`, press `Tab`, and run. Expect `Done. 1 resized` and an `-edited` copy whose longest side is 128 pixels. A second run writes `-edited-1`.
- **Convert format.** In `Convert Image Format`, choose `PNG` under `Convert to` and choose `Convert`. Expect `Done. 1 converted`, `Saved as png`, and `<name>-converted.png`.

## Gotchas

- Steps run in a fixed order whatever order you add them: Crop, Resize, Rotate and Flip, Colour and Tone, Watermark, Remove Metadata.
- A cleared `Longest side, pixels` field keeps a `0`, so typing gives `0128`. Toolbox reads it as 128.
- The message `Longest side must be between 1 and 16384 pixels.` can stay visible after a valid value is committed. Check the run result instead.
