# Tauri tool matrix

The [Tauri registry](../ToolboxTauri/src/registry/index.ts) defines the catalog,
category, display order, command, and availability for `tauri-port`. It currently
includes document, PDF, and image utilities. OCR PDF, Blur Faces, and Remove
Background are marked unavailable because their offline resources are not bundled.

`implemented` records the application status. It does not establish full parity
with every requirement in the [historical parity specification](tauri-parity-spec.md).
The [verification reference](parity-verification-harness.md) describes the checks
that exist today.

| ID | Category | Status | Command | Verification |
| --- | --- | --- | --- | --- |
| pdf-unlock | Documents | implemented | remove_password | remove-password |
| pdf-page-numbers | PDF | implemented | add_page_numbers | pdf-page-numbers |
| pdf-merge | PDF | implemented | merge_pdfs | pdf-merge |
| pdf-watermark | PDF | implemented | watermark_pdf | pdf-watermark |
| pdf-crop | PDF | implemented | crop_pdf | pdf-crop |
| pdf-edit | PDF | implemented | edit_pdf | pdf-edit |
| pdf-protect | PDF | implemented | protect_pdf | protect-pdf |
| images-to-pdf | PDF | implemented | images_to_pdf | images-to-pdf |
| pdf-to-images | PDF | implemented | pdf_to_images | pdf-to-images |
| pdf-to-text | PDF | implemented | pdf_to_text | pdf-to-text |
| pdf-split | PDF | implemented | split_pdf | pdf-split |
| pdf-image-extract | PDF | implemented | extract_pdf_images | pdf-image-extract |
| pdf-sign | PDF | implemented | sign_pdf | pdf-sign |
| pdf-ocr | PDF | unavailable | ocr_pdf | pdf-ocr |
| pdf-remove-pages | PDF | implemented | remove_pdf_pages | pdf-remove-pages |
| pdf-extract-pages | PDF | implemented | extract_pdf_pages | pdf-extract-pages |
| pdf-organize | PDF | implemented | organize_pdf | pdf-organize |
| pdf-compress | PDF | implemented | compress_pdf | pdf-compress |
| heic-convert | Images | implemented | convert_images | convert-image-format |
| compress | Images | implemented | compress_images | compress-images |
| resize | Images | implemented | resize_images | resize-images |
| rotate | Images | implemented | rotate_images | rotate-images |
| crop | Images | implemented | crop_images | crop-images |
| icon-set | Images | implemented | generate_icon_set | icon-set |
| gif-create | Images | implemented | create_gif | gif-create |
| gif-extract | Images | implemented | extract_gif_frames | gif-extract |
| image-watermark | Images | implemented | watermark_images | image-watermark |
| image-metadata | Images | implemented | image_metadata | image-metadata |
| image-tone | Images | implemented | adjust_image_tone | image-tone |
| tiff-pages | Images | implemented | process_tiff_pages | tiff-pages |
| image-blur-faces | Images | unavailable | blur_faces | image-blur-faces |
| image-remove-bg | Images | unavailable | remove_image_background | image-remove-bg |

The `pdf-unlock` ID remains stable for saved navigation. Its current title is
**Remove Password**, and it handles PDF, Word, Excel, and PowerPoint documents.
**Edit PDF** adds text, highlights, shapes, and notes through `edit_pdf`.

The three unavailable tools display an explanation before file selection.
The [vision adapter reference](tauri-vision-engines.md) describes the native
commands and their current limitations.

The `Verification` column contains registry identifiers, not proof that a
matching fixture or recipe passed. `npm run check:matrix`, run from
`ToolboxTauri/`, checks registry completeness, unique IDs, and the presence of
command, verification, and view fields. It does not compare a Swift registry or
execute tools. The legacy Swift catalog remains on `main`.
