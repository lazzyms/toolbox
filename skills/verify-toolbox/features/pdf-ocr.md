# OCR PDF

Open `OCR PDF` (blurb: `Read text out of scans through an offline OCR adapter.`), select a scanned PDF, and press `Run offline OCR`. Expect `OCR text extracted in page order` and a `{stem}-ocr-text.txt` beside the source. Verify the text matches the scan and the source is unchanged.

Prerequisites: tesseract and `eng.traineddata` reachable through `TOOLBOX_TESSERACT_PATH` and `TOOLBOX_TESSDATA_PATH` (see the Launch section), a bundled OCR resource, or `tesseract` on `PATH`, plus `pdftoppm` for page rendering. Without them, expect an explicit unavailable result and no output file.
