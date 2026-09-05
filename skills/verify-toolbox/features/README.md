# Toolbox Tauri feature map

The primary surface is the cross-platform Tauri desktop window. Every registry entry below has a dedicated verification recipe and invokes a typed Rust command through Tauri IPC.

| Feature | Entry point | Automated proof |
| --- | --- | --- |
| [Remove Password](unlock-pdf.md) | Documents → Remove Password | PDF and Office password removal |
| [Page numbers](pdf-page-numbers.md) | PDF → Page Numbers | page geometry and overlay |
| [Merge PDF](pdf-merge.md) | PDF → Merge | qpdf output page count |
| [Watermark PDF](pdf-watermark.md) | PDF → Watermark | overlay output and original preservation |
| [Crop PDF](pdf-crop.md) | PDF → Crop | MediaBox and selected scope |
| [Edit PDF](pdf-edit.md) | PDF → Edit | text, notes, highlights, and shapes |
| [Protect PDF](protect-pdf.md) | PDF → Protect | encrypted output round-trip |
| [Images to PDF](images-to-pdf.md) | PDF → Images to PDF | page count and dimensions |
| [PDF to Images](pdf-to-images.md) | PDF → PDF to Images | rendered image output |
| [PDF to Text](pdf-to-text.md) | PDF → PDF to Text | extracted selectable text |
| [Split PDF](pdf-split.md) | PDF → Split | one output per page |
| [Extract PDF images](pdf-image-extract.md) | PDF → Extract Images | original JPEG bytes |
| [Sign PDF](pdf-sign.md) | PDF → Sign | visible overlay output |
| [OCR PDF](pdf-ocr.md) | PDF → OCR | adapter success or explicit unsupported |
| [Remove pages](pdf-remove-pages.md) | PDF → Remove Pages | selected pages absent |
| [Extract pages](pdf-extract-pages.md) | PDF → Extract Pages | selected pages retained |
| [Organize PDF](pdf-organize.md) | PDF → Organize | order/rotation/delete plan |
| [Compress PDF](pdf-compress.md) | PDF → Compress | valid smaller or preserved output |
| [Convert image format](convert-image-format.md) | Images → Convert | valid target bytes |
| [Compress images](compress-images.md) | Images → Compress | no inflation and original preservation |
| [Resize images](resize-images.md) | Images → Resize | expected dimensions |
| [Rotate images](rotate-images.md) | Images → Rotate | expected orientation |
| [Crop images](crop-images.md) | Images → Crop | expected pixel rectangle |
| [App icons](image-icons.md) | Images → Icons | preset outputs |
| [Create GIF](gif-create.md) | Images → GIF Maker | animated frame count |
| [Extract GIF frames](gif-extract.md) | Images → Frames | frame outputs |
| [Image watermark](image-watermark.md) | Images → Watermark | changed output and original preservation |
| [Image metadata](image-metadata.md) | Images → Metadata | output metadata policy |
| [Image tone](image-tone.md) | Images → Tone | changed pixel values |
| [TIFF pages](tiff-pages.md) | Images → TIFF Pages | TIFF output validity |
| [Blur faces](image-blur-faces.md) | Images → Blur Faces | adapter success or explicit unsupported |
| [Remove background](image-remove-bg.md) | Images → Cutout | adapter success or explicit unsupported |

Cross-cutting coverage includes native file selection and drop paths, per-file result isolation, collision-safe output naming, keyboard operation, Windows path display, and `npm run check:release`.
