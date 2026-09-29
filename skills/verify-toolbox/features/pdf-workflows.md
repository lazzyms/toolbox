# PDF workflows

PDF workflows group page editing and conversion actions behind the PDF category and the command center.

## Sub-features

- `pdf-edit` opens the PDF editor for text, highlights, shapes, and notes.
- `pdf-crop` crops PDF page content.
- `pdf-watermark` adds fixed or repeated text watermarks.
- `pdf-sign` places a typed or image signature.
- `pdf-page-numbers` stamps page numbers.
- `pdf-remove-pages` removes selected pages.
- `pdf-organize` reorders, rotates, removes, or inserts pages.
- `pdf-to-images` renders PDF pages as image files.
- `pdf-to-text` extracts selectable text.
- `pdf-image-extract` extracts embedded JPEG images.
- `images-to-pdf` combines selected images into a PDF.
- `pdf-ocr` reads text from scanned pages when the offline OCR adapter is available.
- `pdf-merge` combines multiple PDFs.
- `pdf-split` creates files from page ranges or chunks.
- `pdf-extract-pages` creates a PDF from selected page ranges.
- `pdf-compress` writes a compressed PDF copy.

## How to get to it (user POV)

- Choose `PDF` under Tool categories and select the desired action.
- Search `Search tools` for an exact action title such as `Edit PDF` or `Merge PDF`.
- Choose an action in Quick access or Recent if it is visible there.
- Choose `Open PDF editor` or `Open PDF conversion` from the corresponding Tool library card.

## Driving it with CUA desktop control

Preconditions:

- The native Toolbox window passes Doctor.
- Copy a supported PDF or image into the run's `scratch/` directory. Keep the original source outside that directory unchanged.
- For page-editor actions, use a PDF with at least one page. For merge or split, prepare the number of files or pages required by the action.

- **Enter a PDF action.** Search its exact title, such as `Edit PDF`, then choose `Open Edit PDF`. The PDF editor workspace appears and the live status reports that it is open.
- **Load a PDF editor input.** Choose `Choose files to process` and select the scratch PDF in the native file picker. The `PDF page canvas` and page thumbnails appear. The `Export PDF` button stays disabled until the preview is ready and an edit can be exported.
- **Edit a page.** Choose a tool in the `PDF editor tools` toolbar, apply it to the canvas, and confirm the resulting object or page change in the preview. Choose `Export PDF`. Confirm the exported copy exists in scratch and the source hash is unchanged.
- **Enter a conversion action.** Search the exact title and choose its `Open <title>` result. Choose `Choose files to process`. The `Open document` region lists the selected file and the action's controls appear.
- **Set conversion options.** Use the visible controls for the selected action. Examples include `Render DPI`, `Output format`, `Page range`, `Split mode`, `Pages per file`, `Page numbers or ranges`, and `Compression quality`.
- **Export a conversion.** Choose `Export <title>`. Wait for processing to finish. The result list reports completed or failed files and names any output. Confirm each reported output exists in scratch and the source hash is unchanged.
- **Check OCR availability.** Choose `OCR PDF` and inspect its capability state before processing. If the offline OCR adapter is unavailable, report that state rather than counting it as a successful OCR run.

## Gotchas

- The `PDF editor tools` toolbar and the page and zoom controls have keyboard navigation. Use their accessible names instead of screen coordinates.
- A page preview can be unavailable or still rendering. `Export PDF` must remain disabled until the preview can be verified.
- The repository does not include a PDF fixture. Create a disposable PDF with the `Images to PDF` action as described in the main skill, or bring a non-sensitive local sample and copy it into scratch.
- OCR depends on the offline OCR adapter. An unavailable adapter is a capability result, not a zero-page result.
- File operations create new outputs. Never select a checked-in asset as the direct input when the operation writes beside its source.
