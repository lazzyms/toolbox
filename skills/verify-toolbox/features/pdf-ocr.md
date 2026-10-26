# OCR PDF

Open `Open OCR PDF` from the PDF category. The tool is registered as implemented, so the dropzone and a run button appear. The button's visible text is `OCR` and its accessible name is `Run offline OCR`.

Select a scanned PDF (100 MiB or less) and click `OCR`. It needs three things: the tesseract engine (bundled `ocr/manifest.json`, `TOOLBOX_TESSERACT_PATH`, or `tesseract` on `PATH`), tessdata (`engTraineddata` or `TOOLBOX_TESSDATA_PATH`), and `pdftoppm` (`TOOLBOX_PDFTOPPM_PATH` or `PATH`). With all three, expect `<name>-ocr-text.txt` and the detail `OCR text extracted in page order`. If any is missing, expect `1 of 1 files failed` with an unavailable message such as `Resource engTraineddata is unavailable.` or `PDF rasterizer is unavailable...`, and no output. A PDF with no readable text fails with `OCR completed but found no readable text.`
