# PDF Editor

The PDF Editor workspace edits pages inside one PDF and runs whole-file tools on one or many PDFs.

## Sub-features

- `edit-pdf` opens one PDF in the page editor with marks, page actions, and export.
- `organize-pdf` reorders, rotates, and deletes pages.
- `merge-pdf` combines PDFs in list order into `<first>-merged.pdf`.
- `split-pdf` writes `-page-N` or `-pages-A-B` files from page ranges or every N pages.
- `crop-resize-pdf` crops or resizes pages into `-cropped` copies.
- `page-numbers` numbers the pages of one PDF.
- `redact-pdf` removes content inside drawn boxes.
- `privacy-check` reports hidden data without changing any PDF.
- `edit-properties` keeps, sets, or clears document properties in an `-edited` copy.
- `rotate-pdfs` rotates every page into `-rotated` copies.
- `repair-pdf` writes `-repaired` copies.
- `compare-pdfs` writes a `-comparison.txt` text report for two PDFs.
- `extract-pages` writes the chosen pages to `-extracted.pdf`.
- `bates-numbering` stamps Bates numbers into `-bates` copies.

## How to get to it (user POV)

- Choose `PDF Editor` in the home rail, or an `Open <tool>` card in the Tool library.
- Switch tools with the toolbar items `Edit PDF`, `Organize PDF`, `Merge PDF`, `Split PDF`, `Crop and Resize PDF`, `Add PDF Page Numbers`, `Redact PDF`, `Privacy Check`, `Edit Properties`, `Rotate PDFs`, `Repair PDF`, `Compare PDFs`, `Extract PDF Pages`, and `Bates Numbering`.

## Driving it with CUA desktop control

Preconditions:

- Two multi-page PDF fixture copies are in `scratch/`.
- Single-file tools (`Edit PDF`, `Add PDF Page Numbers`) show `<tool> works on one PDF at a time` when more than one PDF is open. Choose the file with its radio, or clear the list first.

- **Edit PDF.** Open one PDF. The editor shows tool checkboxes (`Organize`, `Select`, `Edit text`, `Text`, `Highlight`, `Shape`, `Signature`, `Fill form`, `Watermark`, `Page numbers`, `Header and footer`, `Crop`, `Redact`), `Page N` checkboxes, `Undo (⌘Z)`, `Redo (⇧⌘Z)`, `Reset…`, a `Save to …` menu, `Export As…`, and `Export PDF`.
- **Merge.** Pick two PDFs and keep `Sort` at `Order added`. Choose `Export Merge PDF`, then confirm `Save the merged PDF` points at `scratch/` and choose `Save`. Expect `Done. 2 merged` and `<first>-merged.pdf` whose page count is the sum.
- **Split.** Choose `Page ranges`, type `1, 2-3` in `Page ranges`, press `Tab`, and choose `Export Split PDF`. Expect `Done. 2 split` and `-page-1.pdf` and `-pages-2-3.pdf` per input.
- **Rotate.** Keep `Rotation direction` at `Clockwise 90°` and choose `Export Rotate PDFs`. Expect `Done. 2 rotated` and `-rotated.pdf` copies.
- **Repair.** Choose `Export Repair PDF`. Expect `Done. 2 repaired`, `Repaired`, and `-repaired.pdf` copies that pass `qpdf --check`.
- **Privacy Check.** Choose `Check 2 PDFs`. Expect `Done. 2 reports` and `Nothing was changed in any PDF.` No file is written.
- **Page numbers.** With one PDF open, choose `Add PDF Page Numbers` and run it. Expect `Done. 1 numbered`.
- **Extract pages.** Type `1, 3` in `Page numbers or ranges` and choose `Export Extract PDF Pages`. Expect `Done. 1 extracted` and a two-page `-extracted.pdf`.
- **Bates.** Type `VER-` in `Prefix`, keep `Start at` and `Digits`, and choose `Number 1 file`. Expect `-bates.pdf`, and check its text with `pdftotext` for the `VER-` prefix.
- **Edit Properties.** Choose `Set` for the title, type a value in `New Title`, and choose `Save edited copy`. Expect `Done. 1 saved` and an `-edited.pdf` whose `pdfinfo` title matches.
- **Compare.** Open exactly two PDFs, the original and the new version, and choose `Export Compare PDFs`. Confirm `Save the comparison report` points at `scratch/`. Expect `Done. 1 compared`, `No text differences found` for identical text, and a `-comparison.txt` report.
- **Crop.** Choose `Crop pages` and `Rectangle`, edit `Left`, `Top`, `Width`, and `Height`, keep `Pages` at `All pages`, and choose `Crop 2 PDFs`. Expect `Done. 2 cropped` and `-cropped.pdf` copies with the new crop box.
- **Redact.** `Export Redact PDF` stays disabled until you draw at least one box on the page with a foreground mouse drag.

## Gotchas

- The file list is shared across the workspace. Choose `Clear list` before a tool that needs a different input count.
- The Compare tool's example line names a `.pdf`, but the report is a `.txt` file.
- The Split example line can lag behind the typed ranges. Check the output names on disk.
