# Media

The Media workspace makes app icons and GIFs, extracts GIF frames, splits or combines TIFF pages, and holds the offline vision tools.

## Sub-features

- `app-icons` writes an icon set, such as `-macos-icon-16.png` through `-macos-icon.icns`.
- `create-gif` combines images into one `-animated.gif`.
- `extract-gif-frames` writes `-frame-NNN.png` files and a `-timings.json`.
- `split-combine-tiff` writes `-page-NN.tif` files or one combined TIFF.
- `blur-faces` and `remove-background` need the optional vision tools.

## How to get to it (user POV)

- Choose `Media` in the home rail, or an `Open <tool>` card in the Tool library.
- Switch tools with the toolbar items `Generate App Icons`, `Create GIF`, `Extract GIF Frames`, `Split and Combine TIFF`, `Blur Faces`, and `Remove Background`.

## Driving it with CUA desktop control

Preconditions:

- A square PNG and a two-page TIFF are in `scratch/`.
- Choose `Clear list` between tools. The list is shared across the workspace.

- **App icons.** Keep `Icon preset` at `macOS` and choose `Icons`. Expect `Done. 1 generated`, `macOS PNG and ICNS icon set saved.`, six PNG sizes, and one `.icns`.
- **Create GIF.** Pick two images. Each appears as a `Frame N` checkbox. Type a value in `Frame delay in milliseconds`, keep `Loop animation`, and choose `Make GIF…`. Confirm `Save the GIF` points at `scratch/` and choose `Save`. Expect `Done. 1 created`, `GIF saved`, and `-animated.gif`. A 250 ms delay is stored as 25 hundredths per frame, and the file has a loop extension.
- **Extract GIF Frames.** Pick the new GIF and choose `Frames`. Expect `Done. 1 extracted`, `2 frames saved.`, `-frame-001.png`, `-frame-002.png`, and a `-timings.json` with each frame's `delayMs`.
- **Split TIFF.** Pick the two-page TIFF, choose `Split pages`, and choose `Split 1 TIFF`. Expect `Done. 1 processed`, `TIFF pages saved`, and `-page-01.tif` and `-page-02.tif` with one page each.
- **Vision tools.** `Blur Faces` and `Remove Background` show as disabled toolbar items when the vision tools are not installed. Record them as unavailable with that prerequisite.

## Gotchas

- The Generate App Icons example line shows `icon-icon.png`, but the outputs are named `<name>-macos-icon-<size>.png`.
- `Make GIF…` opens an in-app save dialog. Check its `Save to` path before you save.
