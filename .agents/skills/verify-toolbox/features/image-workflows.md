# Image workflows

Image workflows edit images and provide utilities for icon sets, animation frames, metadata, and TIFF pages. They live in the Image Editor and Media views. Compress Images lives in the Compress Files view.

## Sub-features

- `image-convert` converts image formats.
- `image-compress` compresses one or more images.
- `image-resize` scales images to chosen dimensions.
- `image-rotate` rotates or flips images.
- `image-crop` crops images to an aspect ratio or a free rectangle.
- `image-watermark` adds a visible text watermark.
- `image-tone` adjusts brightness, contrast, saturation, and exposure.
- `icon-set` creates app icons from one image.
- `gif-create` creates a GIF from ordered images.
- `gif-extract` writes GIF frames as image files.
- `tiff-pages` splits or combines TIFF pages.
- `image-metadata` inspects metadata or saves a copy without it.
- `image-blur-faces` and `image-remove-bg` are listed but unavailable in every build.

## How to get to it (user POV)

- Choose `Image Editor`, `Media`, or `Compress Files` under Views in the sidebar and select the action in the view's tool rail.
- Image Metadata sits in the Image Editor rail but uses the Media layout.
- Search `Search tools` for an exact action title such as `Compress Images` or `Create GIF` and choose `Open <title>`.
- Choose `Open Image Editor` or `Open Media` from the corresponding Tool library card.

## Driving it with CUA desktop control

Preconditions:

- The native Toolbox window passes Doctor.
- Make a scratch image with `sips -s format png /System/Library/CoreServices/CoreTypes.bundle/Contents/Resources/GenericDocumentIcon.icns --out <run-dir>/scratch/sample.png`. Never process a system or checked-in icon directly.
- Settings > Output location is `Next to each original`.
- For GIF creation, stage at least two ordered images. For GIF or TIFF frame actions, use a matching local sample.

- **Enter an image action.** Search its exact title and choose `Open <title>`. Confirm the workspace heading, such as "Image Editor". An unavailable tool shows its own title instead.
- **Load an image.** Choose `Choose files to process` and select the scratch copy through the native file picker. The `Files` list shows the selected file.
- **Edit an image.** The edit tabs are Resize, Rotate, Crop, Watermark, and Adjust. Use the controls named `Resize mode`, `Width`, `Height`, `Rotation`, `Mirror`, `Aspect ratio`, `Watermark text`, `Brightness`, `Contrast`, `Saturation`, and `Exposure`. `Crop width` and `Crop height` appear only when `Aspect ratio` is "Free crop". The flip control's visible text is "Flip" while its name is `Mirror`.
- **Convert.** There is no per-tool convert button. Set `Target format`, then choose `Export edited images`. The current draft counts without `Add edit to plan`: the plan reads "0 committed edits plus current draft" and "1 edits will be applied". On `sample.png` with JPEG the result reads "Done. 1 converted" and writes `sample-edited.jpg`. Proven 2026-10-23.
- **Export a combined edit plan.** Add each edit with `Add edit to plan`, confirm the preview updates, then choose `Export edited images`. Verify the output and the unchanged source hash.
- **Compress.** Open `Compress Images` in Compress Files. Set `Quality` or `Preserve original pixels`, then choose `Export Compress Images` (visible text "Compress"). The result names each output.
- **Generate or inspect media.** Use the visible controls for the icon, GIF, TIFF, or metadata action. Create GIF and TIFF combine open a save dialog. The result names each output. Metadata inspection shows a report without writing a file.
- **Check vision tools.** The `Blur Faces` and `Remove Background` rail items are disabled and marked "Unavailable". Opening one shows "Unavailable in this build." and its search help reads "<title> is unavailable in this build". Record them as unavailable.

## Gotchas

- Multiple-file operations can preserve selected file order. Check the file list before creating a GIF or combined TIFF.
- Image edit plans need a valid preview before export. Do not count a disabled `Export edited images` button as a failed native command.
- Convert's Example line shows `sample-converted.png` while it writes `sample-edited.jpg`. Trust the result row, then check disk.
- Metadata inspection does not write a file. Metadata removal writes a new copy.
- The native output is written beside the scratch input.
