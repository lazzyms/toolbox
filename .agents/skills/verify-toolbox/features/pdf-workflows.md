# PDF workflows

PDF workflows group page editing and conversion actions under the PDF Editor and PDF Conversion views. Compress PDF lives in the Compress Files view.

## Sub-features

- `pdf-edit` opens the PDF editor for text, highlights, shapes, and notes.
- `pdf-crop` crops PDF page content.
- `pdf-watermark` adds fixed or repeated text watermarks.
- `pdf-sign` places a typed or image signature.
- `pdf-page-numbers` stamps page numbers.
- `pdf-remove-pages` removes selected pages.
- `pdf-organize` reorders, rotates, removes, or inserts pages.
- `pdf-merge` combines multiple PDFs.
- `pdf-split` creates files from page ranges or chunks.
- `pdf-redact` blacks out a dragged area.
- `pdf-rotate` rotates every page of each PDF.
- `pdf-repair` saves a clean copy of a damaged PDF.
- `pdf-compare` writes a text report comparing two PDFs.
- `pdf-extract-pages` creates a PDF from selected page ranges.
- `pdf-to-images` renders PDF pages as image files.
- `pdf-to-text` extracts selectable text.
- `pdf-image-extract` extracts embedded JPEG images.
- `images-to-pdf` combines selected images into a PDF.
- `pdf-ocr` is listed but unavailable in every build.
- `pdf-compress` writes a compressed PDF copy.

## How to get to it (user POV)

- Choose `PDF Editor`, `PDF Conversion`, or `Compress Files` under Views in the sidebar and select the action in the view's tool rail.
- PDF Editor lists Edit PDF (with Watermark, Sign, and Remove Pages), Organize, Merge, Split, Crop, Page Numbers, and Redact. Rotate PDFs, Repair PDF, Compare PDFs, and Extract PDF Pages sit under "More".
- PDF Conversion lists PDF to Images, Images to PDF, OCR PDF, PDF to Text, and Extract Images from PDF.
- Search `Search tools` for an exact action title such as `Edit PDF` or `Merge PDF` and choose `Open <title>`.
- Choose `Open PDF Editor` or `Open PDF Conversion` from the corresponding Tool library card.

## Driving it with CUA desktop control

Preconditions:

- The native Toolbox window passes Doctor.
- `helpers/make-fixtures.sh` has written `plain.pdf` into scratch. Keep the original source outside that directory unchanged.
- Settings > Output location is `Next to each original`. The page editor export fails otherwise.
- For merge or compare, prepare the number of files the action needs.

- **Enter a PDF action.** Search its exact title, such as `Edit PDF`, then choose `Open Edit PDF`. The workspace appears and the live status reports "<View> workspace open."
- **Load a PDF editor input.** Choose the `Choose files to process` inside the `Open document` region and select the scratch PDF. The `PDF page canvas` and page thumbnails appear. `Export PDF` stays disabled until the preview is ready and an edit can be exported.
- **Edit a page.** Choose a tool in the `PDF editor tools` toolbar, apply it to the canvas, and confirm the change in the preview. Choose `Export PDF`. Confirm the exported copy exists in scratch and the source hash is unchanged.
- **Enter a conversion action.** Search the exact title and choose `Open <title>`. Choose `Choose files to process`. The `Files` list shows the selected file and the action's controls appear.
- **Set conversion options.** Controls go by accessible name: `Render DPI`, `Output format`, `Page range`, `Split mode`, `Page ranges`, `Pages per file`, `Page numbers or ranges`, `Rotation direction`, `Redaction area`, and `Compression quality`. Visible labels can be shorter, such as "Resolution" or "Direction".
- **Export a conversion.** Choose the button named `Export <title>`, such as `Export PDF to Text`. Wait for the headline "Done. N <verb>", such as "Done. 1 extracted". Each row names its output and has `Open <file>` and `Show in Finder: <file>`. Confirm the output exists in scratch and the source hash is unchanged.
- **PDF to Text.** On `plain.pdf` the result is `plain-text.txt` containing "Toolbox verification". Proven 2026-10-23.
- **Check OCR.** The `OCR PDF` rail item is disabled and marked "Unavailable". Opening it from search shows "Unavailable in this build." Record it as unavailable, not as an OCR run.

## Gotchas

- The `PDF editor tools` toolbar and the page and zoom controls have keyboard navigation. Use their accessible names instead of screen coordinates.
- A page preview can be unavailable or still rendering. `Export PDF` must remain disabled until the preview can be verified.
- With no file open, the page editor shows two `Choose files to process` buttons. Use the one in the `Open document` region. After a file loads its visible text becomes "Replace file" or "Add files" while the name stays.
- The `Export <title>` visible text differs from its name. Merge, Images to PDF, and Compare show "Merge…", "Make PDF…", and "Compare…" and open a save dialog instead of writing beside the input.
- The page editor writes `<stem>-edited-1.pdf` while the Example line shows `<stem>-edited.pdf`. PDF to Text's Example shows `plain-text.pdf` while it writes `plain-text.txt`. Trust the result row, then check disk.
- OCR PDF's message says the offline resource "is not bundled". Report it as unavailable regardless.
- File operations create new outputs. Never select a checked-in asset as the direct input when the operation writes beside its source.
