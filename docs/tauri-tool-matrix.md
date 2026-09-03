# Tauri tool matrix

`Sources/Toolbox/Registry/Utility+PDF.swift` and `Utility+Images.swift` remain the canonical catalog. `ToolboxTauri/src/registry/index.ts` mirrors their order and records the Tauri migration status.

| ID | Category | Status | Command | Verification |
| --- | --- | --- | --- | --- |
| pdf-unlock | PDF | implemented | unlock_pdf | unlock-pdf |
| pdf-page-numbers | PDF | planned | add_page_numbers | pdf-page-numbers |
| pdf-merge | PDF | planned | merge_pdfs | pdf-merge |
| pdf-watermark | PDF | planned | watermark_pdf | pdf-watermark |
| pdf-crop | PDF | planned | crop_pdf | pdf-crop |
| pdf-protect | PDF | implemented | protect_pdf | protect-pdf |
| images-to-pdf | PDF | planned | images_to_pdf | images-to-pdf |
| pdf-to-images | PDF | planned | pdf_to_images | pdf-to-images |
| pdf-to-text | PDF | planned | pdf_to_text | pdf-to-text |
| pdf-split | PDF | planned | split_pdf | pdf-split |
| pdf-image-extract | PDF | planned | extract_pdf_images | pdf-image-extract |
| pdf-sign | PDF | planned | sign_pdf | pdf-sign |
| pdf-ocr | PDF | planned | ocr_pdf | pdf-ocr |
| pdf-remove-pages | PDF | planned | remove_pdf_pages | pdf-remove-pages |
| pdf-extract-pages | PDF | planned | extract_pdf_pages | pdf-extract-pages |
| pdf-organize | PDF | planned | organize_pdf | pdf-organize |
| pdf-compress | PDF | planned | compress_pdf | pdf-compress |
| heic-convert | Images | implemented | convert_images | convert-image-format |
| compress | Images | implemented | compress_images | compress-images |
| resize | Images | planned | resize_images | resize-images |
| rotate | Images | planned | rotate_images | rotate-images |
| crop | Images | planned | crop_images | crop-images |
| icon-set | Images | planned | generate_icon_set | icon-set |
| gif-create | Images | planned | create_gif | gif-create |
| gif-extract | Images | planned | extract_gif_frames | gif-extract |
| image-watermark | Images | planned | watermark_images | image-watermark |
| image-metadata | Images | planned | image_metadata | image-metadata |
| image-tone | Images | planned | adjust_image_tone | image-tone |
| tiff-pages | Images | planned | process_tiff_pages | tiff-pages |
| image-blur-faces | Images | planned | blur_faces | image-blur-faces |
| image-remove-bg | Images | planned | remove_image_background | image-remove-bg |

Run `npm run check:matrix` from `ToolboxTauri` to compare IDs, order, and required contract fields against the Swift registry.
