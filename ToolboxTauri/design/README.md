# Toolbox UI design

This folder is the source of truth for the Toolbox app UI. It is a checked-in
snapshot of the approved design canvas for issue lazzyms/toolbox#429:
https://claude.ai/artifact/34yHEsXnbDuExy8brzbLhe (private to the owner).

## What the design decides

The boards decide what the UI shows and does:

- Layout, navigation, and the order of controls.
- Every state a view can be in: empty, loading, running, results, errors, and dialogs.
- All visible text: labels, buttons, hints, errors, and status messages.
- Interaction: mouse handles, drag behavior, keyboard shortcuts, and focus order.
- Accessibility: names, roles, and keyboard alternatives to mouse actions.
- Differences between macOS and Windows, and between dark and light themes.

The rules in [AGENTS.md](../../AGENTS.md) decide how the UI is built. Build each
board with the shadcn/ui components and theme. Use the board's CSS only as a
visual reference; do not copy its styles into the app. If a board cannot be
built within those rules, raise it and do not change the design on your own.

## Change the UI

1. Change the canvas first and get it approved.
2. Copy the changed boards into `boards/`.
3. Run `npm run design:screens` from `ToolboxTauri/` to refresh `screens/`.
4. Implement the change, with automated coverage for each changed behavior.

When the app and a board disagree, the board is correct. Fix the app, or get
the board changed first.

## Read the design

- `screens/` has a picture of every board state. `screens/index.json` maps
  each picture to its board and state.
- `boards/` has the boards as HTML. Open a board in a browser to click
  through it. Its links go to the other boards. A board's states are set in
  the address, for example `PdfSign.dc.html?drag=Rotating&theme=light&platform=Windows`.
- `boards/canvas.json` is the layout of the canvas.
- `boards/support.js` is a small local stand-in for the canvas runtime. It
  fills in a board's state from the address.

## Boards

### Command center and shared batch flow

| Board | What it shows | States |
|---|---|---|
| [Main](boards/Main.dc.html) | Command center: six views | [default](screens/Main--default.jpg) |
| [Empty](boards/Empty.dc.html) | Batch tool: empty (drop target) | [default](screens/Empty--default.jpg), [dragging-true](screens/Empty--dragging-true.jpg) |
| [Files](boards/Files.dc.html) | Files added and options | [default](screens/Files--default.jpg), [showPassword-true](screens/Files--showPassword-true.jpg) |
| [Destination](boards/Destination.dc.html) | Per-run destination and folder-missing prompt | [default](screens/Destination--default.jpg), [showPrompt-false](screens/Destination--showPrompt-false.jpg) |
| [Running](boards/Running.dc.html) | Running, locked, cancel confirmation | [default](screens/Running--default.jpg), [showCancel-false](screens/Running--showCancel-false.jpg) |
| [Results](boards/Results.dc.html) | Results: succeeded, skipped, failed | [default](screens/Results--default.jpg), [filter-needs-attention](screens/Results--filter-needs-attention.jpg), [filter-warnings](screens/Results--filter-warnings.jpg) |
| [Cancelled](boards/Cancelled.dc.html) | Cancelled batch | [default](screens/Cancelled--default.jpg) |
| [LargeBatch](boards/LargeBatch.dc.html) | Large batch with many warnings | [default](screens/LargeBatch--default.jpg), [expandAll-true](screens/LargeBatch--expandAll-true.jpg) |
| [RowPatterns](boards/RowPatterns.dc.html) | Result row patterns | [default](screens/RowPatterns--default.jpg) |

### PDF Editor

| Board | What it shows | States |
|---|---|---|
| [PdfOpen](boards/PdfOpen.dc.html) | Open a PDF: empty, several dropped, locked, signed | [default](screens/PdfOpen--default.jpg), [state-several-dropped](screens/PdfOpen--state-several-dropped.jpg), [state-locked-file](screens/PdfOpen--state-locked-file.jpg), [state-signed-file](screens/PdfOpen--state-signed-file.jpg) |
| [PdfText](boards/PdfText.dc.html) | Viewer with edit toolbar: Add text | [default](screens/PdfText--default.jpg), [editing-false](screens/PdfText--editing-false.jpg), [scriptPackPrompt-true](screens/PdfText--scriptPackPrompt-true.jpg), [drag-moving](screens/PdfText--drag-moving.jpg), [drag-resizing](screens/PdfText--drag-resizing.jpg) |
| [PdfSign](boards/PdfSign.dc.html) | Sign: typed or picture signature | [default](screens/PdfSign--default.jpg), [source-picture](screens/PdfSign--source-picture.jpg), [ink-black](screens/PdfSign--ink-black.jpg), [drag-moving](screens/PdfSign--drag-moving.jpg), [drag-resizing](screens/PdfSign--drag-resizing.jpg), [drag-rotating](screens/PdfSign--drag-rotating.jpg) |
| [PdfForms](boards/PdfForms.dc.html) | Fill forms and flatten | [default](screens/PdfForms--default.jpg), [flatten-confirming](screens/PdfForms--flatten-confirming.jpg), [flatten-on](screens/PdfForms--flatten-on.jpg) |
| [PdfRedact](boards/PdfRedact.dc.html) | Redact: mark, then the export disclosure | [default](screens/PdfRedact--default.jpg), [step-export-disclosure](screens/PdfRedact--step-export-disclosure.jpg) |
| [PdfMarks](boards/PdfMarks.dc.html) | Watermark, page numbers, header and footer | [default](screens/PdfMarks--default.jpg), [tool-page-numbers](screens/PdfMarks--tool-page-numbers.jpg), [tool-header-and-footer](screens/PdfMarks--tool-header-and-footer.jpg), [drag-moving](screens/PdfMarks--drag-moving.jpg), [drag-resizing](screens/PdfMarks--drag-resizing.jpg), [drag-rotating](screens/PdfMarks--drag-rotating.jpg) |
| [PdfEditText](boards/PdfEditText.dc.html) | Edit existing text: substituted font, too long | [default](screens/PdfEditText--default.jpg), [state-too-long](screens/PdfEditText--state-too-long.jpg) |
| [PdfOrganize](boards/PdfOrganize.dc.html) | Organize pages: grid with page actions | [default](screens/PdfOrganize--default.jpg) |
| [PdfExport](boards/PdfExport.dc.html) | Export a copy: review, progress, done, failed, closing | [default](screens/PdfExport--default.jpg), [step-exporting](screens/PdfExport--step-exporting.jpg), [step-done](screens/PdfExport--step-done.jpg), [step-failed](screens/PdfExport--step-failed.jpg), [step-closing-with-changes](screens/PdfExport--step-closing-with-changes.jpg) |
| [PdfNarrow](boards/PdfNarrow.dc.html) | Narrow window: inspector as a drawer | [default](screens/PdfNarrow--default.jpg), [inspector-closed](screens/PdfNarrow--inspector-closed.jpg) |
| [PdfCrop](boards/PdfCrop.dc.html) | Crop and resize pages | [default](screens/PdfCrop--default.jpg), [mode-resize-pages](screens/PdfCrop--mode-resize-pages.jpg), [drag-moving](screens/PdfCrop--drag-moving.jpg), [drag-resizing](screens/PdfCrop--drag-resizing.jpg) |
| [PdfDetails](boards/PdfDetails.dc.html) | Details: privacy report and properties | [default](screens/PdfDetails--default.jpg), [tab-properties](screens/PdfDetails--tab-properties.jpg) |

### PDF tools and conversion

| Board | What it shows | States |
|---|---|---|
| [Split](boards/Split.dc.html) | Split: page ranges or every N pages | [default](screens/Split--default.jpg), [mode-every-n-pages](screens/Split--mode-every-n-pages.jpg) |
| [Merge](boards/Merge.dc.html) | Merge: ordered files and save dialog | [default](screens/Merge--default.jpg), [step-save-dialog](screens/Merge--step-save-dialog.jpg) |
| [Bates](boards/Bates.dc.html) | Bates numbering: ready and done | [default](screens/Bates--default.jpg), [step-done](screens/Bates--step-done.jpg) |
| [ConvertBatch](boards/ConvertBatch.dc.html) | Convert: PDF to images, extract, EPUB, to PDF | [default](screens/ConvertBatch--default.jpg), [tool-extract-from-pdf](screens/ConvertBatch--tool-extract-from-pdf.jpg), [tool-pdf-to-epub](screens/ConvertBatch--tool-pdf-to-epub.jpg), [tool-convert-to-pdf](screens/ConvertBatch--tool-convert-to-pdf.jpg) |
| [OCR](boards/OCR.dc.html) | OCR: language packs and output | [default](screens/OCR--default.jpg), [pack-downloading](screens/OCR--pack-downloading.jpg) |
| [ImagesToPdf](boards/ImagesToPdf.dc.html) | Images to PDF: ordered pages and save dialog | [default](screens/ImagesToPdf--default.jpg) |
| [FormattedPdf](boards/FormattedPdf.dc.html) | Formatted PDF: blocks, preview, draft | [default](screens/FormattedPdf--default.jpg), [draft-restored](screens/FormattedPdf--draft-restored.jpg) |

### Image Editor, compression and media

| Board | What it shows | States |
|---|---|---|
| [ImageEdit](boards/ImageEdit.dc.html) | Image Editor: resize, crop, rotate | [default](screens/ImageEdit--default.jpg), [edit-resize](screens/ImageEdit--edit-resize.jpg), [edit-rotate](screens/ImageEdit--edit-rotate.jpg), [edit-adjust](screens/ImageEdit--edit-adjust.jpg), [edit-watermark](screens/ImageEdit--edit-watermark.jpg), [drag-moving](screens/ImageEdit--drag-moving.jpg), [drag-resizing](screens/ImageEdit--drag-resizing.jpg) |
| [ImageConvert](boards/ImageConvert.dc.html) | Convert image format | [default](screens/ImageConvert--default.jpg), [format-png](screens/ImageConvert--format-png.jpg) |
| [Compress](boards/Compress.dc.html) | Compress PDFs and images | [default](screens/Compress--default.jpg) |
| [CompressResults](boards/CompressResults.dc.html) | Compress results: saved, kept, failed | [default](screens/CompressResults--default.jpg) |
| [AppIcons](boards/AppIcons.dc.html) | App icon set: good and non-square source | [default](screens/AppIcons--default.jpg), [source-not-square](screens/AppIcons--source-not-square.jpg) |
| [CreateGif](boards/CreateGif.dc.html) | Create GIF: frames, preview, save | [default](screens/CreateGif--default.jpg) |
| [ExtractGif](boards/ExtractGif.dc.html) | Extract GIF frames | [default](screens/ExtractGif--default.jpg) |
| [TiffPages](boards/TiffPages.dc.html) | TIFF Pages: split or combine | [default](screens/TiffPages--default.jpg), [mode-combine](screens/TiffPages--mode-combine.jpg) |

### File Security

| Board | What it shows | States |
|---|---|---|
| [SecEmpty](boards/SecEmpty.dc.html) | Lock & Unlock: empty, mode switch, drop | [default](screens/SecEmpty--default.jpg), [mode-unlock](screens/SecEmpty--mode-unlock.jpg), [dragging-true](screens/SecEmpty--dragging-true.jpg), [showPassword-true](screens/SecEmpty--showPassword-true.jpg) |
| [SecFiles](boards/SecFiles.dc.html) | Files added: flagged rows and not added | [default](screens/SecFiles--default.jpg), [mode-unlock](screens/SecFiles--mode-unlock.jpg), [showPassword-true](screens/SecFiles--showPassword-true.jpg) |
| [SecOptions](boards/SecOptions.dc.html) | Own password, confirm mismatch, per-run folder | [default](screens/SecOptions--default.jpg), [mode-unlock](screens/SecOptions--mode-unlock.jpg), [override-set](screens/SecOptions--override-set.jpg), [override-cleared](screens/SecOptions--override-cleared.jpg), [confirm-match](screens/SecOptions--confirm-match.jpg), [batchPassword-false](screens/SecOptions--batchPassword-false.jpg), [showPassword-true](screens/SecOptions--showPassword-true.jpg) |
| [SecRunning](boards/SecRunning.dc.html) | Running: mode and passwords locked | [default](screens/SecRunning--default.jpg), [mode-unlock](screens/SecRunning--mode-unlock.jpg) |
| [SecResults](boards/SecResults.dc.html) | Results: every lock and unlock outcome | [default](screens/SecResults--default.jpg), [mode-unlock](screens/SecResults--mode-unlock.jpg), [filter-needs-attention](screens/SecResults--filter-needs-attention.jpg), [filter-warnings](screens/SecResults--filter-warnings.jpg), [retryOpen-true](screens/SecResults--retryOpen-true.jpg) |

### App settings and identity

| Board | What it shows | States |
|---|---|---|
| [Settings](boards/Settings.dc.html) | Settings: six sections, use the Section tweak | [default](screens/Settings--default.jpg), [section-output-location](screens/Settings--section-output-location.jpg), [section-packs](screens/Settings--section-packs.jpg), [section-privacy](screens/Settings--section-privacy.jpg), [section-updates](screens/Settings--section-updates.jpg), [section-about-and-support](screens/Settings--section-about-and-support.jpg), [update-ready](screens/Settings--update-ready.jpg) |
| [Packs](boards/Packs.dc.html) | Packs: downloading, failed, offline, low space | [default](screens/Packs--default.jpg), [state-offline](screens/Packs--state-offline.jpg), [state-low-disk-space](screens/Packs--state-low-disk-space.jpg) |
| [Shortcuts](boards/Shortcuts.dc.html) | Keyboard shortcuts for this platform | [default](screens/Shortcuts--default.jpg) |
| [Logo](boards/Logo.dc.html) | App icon: sizes, platforms, in use | [default](screens/Logo--default.jpg) |
