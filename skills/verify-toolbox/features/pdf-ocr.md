# OCR PDF

Open `OCR` from the PDF tools. In the current build the registry marks this
feature unavailable, so the route ends at `Unavailable in this build.` and no
file chooser is shown. Verify that state and that no file is selected or
processed. The prerequisite for a producing drive is a configured offline
OCR adapter (`TOOLBOX_TESSERACT_PATH`, a bundled `vision/manifest.json`
resource, or `tesseract` on `PATH`); in a build with that resource, select a
scan and verify `-ocr-text.txt` output, or an explicit unavailable result with
no output when the adapter is absent.
