# Image workflows

Image workflows edit images and provide utilities for icon sets, animation frames, metadata, TIFF pages, and optional offline vision tasks.

## Sub-features

- `image-convert` converts image formats.
- `image-compress` compresses one or more images.
- `image-resize` scales images to chosen dimensions.
- `image-rotate` rotates or flips images.
- `image-crop` crops images to a rectangle or aspect ratio.
- `image-watermark` adds a visible text watermark.
- `image-tone` adjusts brightness, contrast, saturation, and exposure.
- `icon-set` creates app icons from one image.
- `gif-create` creates a GIF from ordered images.
- `gif-extract` writes GIF frames as image files.
- `tiff-pages` splits or combines TIFF pages.
- `image-metadata` inspects metadata or saves a copy without it.
- `image-blur-faces` blurs faces when the offline vision adapter is available.
- `image-remove-bg` removes an image background when the offline vision adapter is available.

## How to get to it (user POV)

- Choose `Images` under Tool categories and select the desired action.
- Search `Search tools` for an exact action title such as `Compress Images` or `Create GIF`.
- Choose a visible action in Quick access or Recent.
- Choose `Open Image editor` or `Open Media utilities` from the corresponding Tool library card.

## Driving it with CUA desktop control

Preconditions:

- The native Toolbox window passes Doctor.
- Copy supported source images into the run's `scratch/` directory. Keep the checked-in source unchanged.
- For GIF creation, stage at least two ordered images. For GIF or TIFF frame actions, use a matching local sample.

- **Enter an image action.** Search its exact title and choose `Open <title>`. Confirm the Image editor or Media utilities workspace title.
- **Load an image.** Choose `Choose files to process` and select the scratch copy through the native file picker. The `Open document` region lists the selected file.
- **Convert or compress.** For `Convert Image Format`, set `Target format`. For `Compress Images`, set `Preserve original pixels` or `Compression quality`. Choose the action's visible primary button. The result list reports completion and names the output.
- **Resize, rotate, crop, watermark, or adjust tone.** Use the controls named `Resize mode`, `Width`, `Height`, `Rotation`, `Mirror`, `Crop mode`, `Crop width`, `Crop height`, `Watermark text`, `Brightness`, `Contrast`, `Saturation`, and `Exposure` as required. Choose the visible primary action and verify the output file.
- **Export a combined edit plan.** Add an edit with `Add edit to plan`. Confirm the preview updates, then choose `Export edited images`. Verify the output and unchanged source hash.
- **Generate or inspect media.** Use the visible controls for the selected icon, GIF, TIFF, metadata, or vision action. The completed result names each output. For metadata inspection, confirm the report appears without claiming a file was written.
- **Check optional vision tools.** Inspect availability before using `Blur Faces` or `Remove Background`. Report an unavailable adapter as blocked for that workflow.

## Gotchas

- Multiple-file operations can preserve selected file order. Check the file list before creating a GIF or combined TIFF.
- Image edit plans need a valid preview before export. Do not count a disabled `Export edited images` button as a failed native command.
- Metadata inspection does not write a file. Metadata removal writes a new copy.
- Face blur and background removal require the offline vision adapter. They may be unavailable in a development checkout.
- The native output is written beside the scratch input. Do not process a checked-in icon directly.
