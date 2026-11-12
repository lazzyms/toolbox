# PDF Conversion

The PDF Conversion workspace turns PDFs into images or text, images into one PDF, scanned PDFs into searchable PDFs, and documents into PDFs.

## Sub-features

- `pdf-to-images` renders pages to `-page-NN.png`, JPEG, or WebP files.
- `extract-from-pdf` saves `-text.txt` and an `-images` folder.
- `images-to-pdf` combines images into one `-combined.pdf`.
- `ocr-pdf` writes an `-ocr.pdf` searchable PDF or a text file.
- `convert-to-pdf` converts Markdown, HTML, text, and spreadsheet files to PDF.

## How to get to it (user POV)

- Choose `PDF Conversion` in the home rail, or an `Open <tool>` card in the Tool library.
- Switch tools with the toolbar items `PDF to Images`, `Extract from PDF`, `Images to PDF`, `OCR PDF`, and `Convert to PDF`.

## Driving it with CUA desktop control

Preconditions:

- A three-page PDF, a PNG, a Markdown note, an HTML page, and a valid `.xlsx` are in `scratch/`.
- The helper paths from Launch are set. OCR needs tesseract and its English data.

- **PDF to Images.** Choose `PNG`, `150 dpi`, and `All pages`, then `Export PDF to Images`. Expect `Done. 1 converted` and `-page-01.png` through `-page-03.png`.
- **Extract from PDF.** Choose `Both` and `Extract from 1 PDF`. Expect `Done. 1 converted`, a line such as `Saved the text and 1 image`, `-text.txt`, and an `-images` folder.
- **Images to PDF.** Pick a PNG. A PDF left in the shared list is flagged with `1 file won't run with Images to PDF` and `Remove 1 flagged file`. Choose `Match image` and `Export Images to PDF`, confirm `Save the combined PDF` points at `scratch/`, and choose `Save`. Expect `Done. Made 1 PDF from 1 image` and `-combined.pdf`.
- **OCR PDF.** Choose `Searchable PDF` and `Export OCR PDF`. Expect `Done. 1 recognized`, `1 page recognized. English.`, and an `-ocr.pdf` whose `pdftotext` output contains the page text.
- **Convert to PDF.** Pick the Markdown note, the HTML page, and the workbook. Choose `Export Convert to PDF`. Expect one PDF per input. An empty or invalid workbook fails on its own row with `Failed, the input can't be used` while the others succeed, for example `Done. 3 saved, 1 failed`.

## Gotchas

- An unreadable `.xlsx` fails per file. Write a valid one-sheet workbook as the fixture.
- OCR help text mentions an offline OCR adapter. The verification launch points OCR at the local tesseract.
