# Toolbox Tauri feature map

The primary surface is the cross-platform Tauri desktop window. Every registry entry below has a dedicated verification recipe and invokes a typed Rust command through Tauri IPC. The entry point names the category button and the full card title; the card's accessibility name is `Open <card title>`.

| Feature | Entry point | Automated proof |
| --- | --- | --- |
| [Remove Password](unlock-pdf.md) | Documents → Remove Password | PDF and Office password removal |
| [Add PDF Page Numbers](pdf-page-numbers.md) | PDF → Add PDF Page Numbers | page geometry and overlay |
| [Merge PDF](pdf-merge.md) | PDF → Merge PDF | qpdf output page count |
| [Watermark PDF](pdf-watermark.md) | PDF → Watermark PDF | overlay output and original preservation |
| [Crop PDF](pdf-crop.md) | PDF → Crop PDF | MediaBox and selected scope |
| [Edit PDF](pdf-edit.md) | PDF → Edit PDF | typed annotation output and original preservation |
| [Protect PDF](protect-pdf.md) | PDF → Protect PDF | encrypted output round-trip |
| [Images to PDF](images-to-pdf.md) | PDF → Images to PDF | page count and dimensions |
| [PDF to Images](pdf-to-images.md) | PDF → PDF to Images | rendered image output |
| [PDF to Text](pdf-to-text.md) | PDF → PDF to Text | extracted selectable text |
| [Split PDF](pdf-split.md) | PDF → Split PDF | one output per page |
| [Extract Images from PDF](pdf-image-extract.md) | PDF → Extract Images from PDF | original JPEG bytes |
| [Sign PDF](pdf-sign.md) | PDF → Sign PDF | visible overlay output |
| [OCR PDF](pdf-ocr.md) | PDF → OCR PDF | adapter text output or explicit unsupported |
| [Remove PDF Pages](pdf-remove-pages.md) | PDF → Remove PDF Pages | selected pages absent |
| [Extract PDF Pages](pdf-extract-pages.md) | PDF → Extract PDF Pages | selected pages retained |
| [Organize PDF](pdf-organize.md) | PDF → Organize PDF | order/rotation/delete plan |
| [Compress PDF](pdf-compress.md) | PDF → Compress PDF | valid output with preserved geometry |
| [Convert Image Format](convert-image-format.md) | Images → Convert Image Format | valid target bytes |
| [Compress Images](compress-images.md) | Images → Compress Images | no inflation and original preservation |
| [Resize Images](resize-images.md) | Images → Resize Images | expected dimensions |
| [Rotate and Flip Images](rotate-images.md) | Images → Rotate and Flip Images | expected orientation |
| [Crop Images](crop-images.md) | Images → Crop Images | expected pixel rectangle |
| [Generate App Icons](image-icons.md) | Images → Generate App Icons | preset outputs |
| [Create GIF](gif-create.md) | Images → Create GIF | animated frame count |
| [Extract GIF Frames](gif-extract.md) | Images → Extract GIF Frames | frame outputs and timing file |
| [Watermark Images](image-watermark.md) | Images → Watermark Images | changed output and original preservation |
| [Image Metadata](image-metadata.md) | Images → Image Metadata | output metadata policy |
| [Colour and Tone Adjustments](image-tone.md) | Images → Colour and Tone Adjustments | changed pixel values |
| [Split and Combine TIFF](tiff-pages.md) | Images → Split and Combine TIFF | TIFF output validity |
| [Blur Faces](image-blur-faces.md) | Images → Blur Faces | adapter output or explicit unsupported |
| [Remove Background](image-remove-bg.md) | Images → Remove Background | adapter output or explicit unsupported |
| [Settings](settings.md) | `Settings` button | theme toggle and app info |

Cross-cutting coverage includes native file selection and drop paths, per-file result isolation, collision-safe output naming, keyboard operation, Windows path display, and `npm run check:release`.
