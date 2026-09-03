import type { ToolDefinition } from "../contracts";

const planned = (tool: Omit<ToolDefinition, "status">): ToolDefinition => ({ ...tool, status: "planned" });
const pdf = (id: string, title: string, shortTitle: string, blurb: string, symbol: string, tint: string, command: string, verification: string, view: ToolDefinition["view"] = "planned") =>
    planned({ id, title, shortTitle, blurb, symbol, tint, category: "PDF", command, verification, view });
const image = (id: string, title: string, shortTitle: string, blurb: string, symbol: string, tint: string, command: string, verification: string, view: ToolDefinition["view"] = "planned") =>
    planned({ id, title, shortTitle, blurb, symbol, tint, category: "Images", command, verification, view });

export const UtilityRegistry: ToolDefinition[] = [
    { id: "pdf-unlock", title: "Remove PDF Password", shortTitle: "Unlock PDF", blurb: "Save an unlocked copy of a PDF you know the password for.", symbol: "lock-open", tint: "#f97316", category: "PDF", command: "unlock_pdf", verification: "unlock-pdf", view: "pdf-unlock", status: "implemented" },
    pdf("pdf-page-numbers", "Add PDF Page Numbers", "Page Numbers", "Stamp page numbers onto a PDF.", "numbers", "#3b82f6", "add_page_numbers", "pdf-page-numbers"),
    pdf("pdf-merge", "Merge PDF", "Merge", "Combine several PDFs into one file.", "files", "#a855f7", "merge_pdfs", "pdf-merge"),
    pdf("pdf-watermark", "Watermark PDF", "Watermark", "Stamp text or an image across PDF pages.", "droplet", "#06b6d4", "watermark_pdf", "pdf-watermark"),
    { id: "pdf-crop", title: "Crop PDF", shortTitle: "Crop", blurb: "Hide content outside a selected page rectangle.", symbol: "crop", tint: "#10b981", category: "PDF", command: "crop_pdf", verification: "pdf-crop", view: "pdf-crop", status: "implemented" },
    { id: "pdf-protect", title: "Protect PDF", shortTitle: "Protect", blurb: "Add a password so only you can open a PDF.", symbol: "lock", tint: "#ef4444", category: "PDF", command: "protect_pdf", verification: "protect-pdf", view: "pdf-protect", status: "implemented" },
    pdf("images-to-pdf", "Images to PDF", "Images to PDF", "Turn photos and scans into one PDF, including HEIC files.", "photo", "#3b82f6", "images_to_pdf", "images-to-pdf"),
    pdf("pdf-to-images", "PDF to Images", "PDF to Images", "Render PDF pages to JPEG or PNG at 72 to 300 dpi.", "photo", "#6366f1", "pdf_to_images", "pdf-to-images"),
    pdf("pdf-to-text", "PDF to Text", "PDF to Text", "Pull text out of a PDF as text or best-effort Markdown.", "file-text", "#6b7280", "pdf_to_text", "pdf-to-text"),
    pdf("pdf-split", "Split PDF", "Split", "Break one PDF into pages, ranges, or fixed-size chunks.", "file-scissors", "#f97316", "split_pdf", "pdf-split"),
    pdf("pdf-image-extract", "Extract Images from PDF", "Extract Images", "Pull embedded pictures out at their original resolution.", "photo-search", "#22c55e", "extract_pdf_images", "pdf-image-extract"),
    { id: "pdf-sign", title: "Sign PDF", shortTitle: "Sign", blurb: "Place a visible typed signature on a PDF page.", symbol: "signature", tint: "#ec4899", category: "PDF", command: "sign_pdf", verification: "pdf-sign", view: "pdf-sign", status: "implemented" },
    pdf("pdf-ocr", "OCR PDF", "OCR", "Read text out of scans on-device and save a text file.", "scan", "#14b8a6", "ocr_pdf", "pdf-ocr"),
    pdf("pdf-remove-pages", "Remove PDF Pages", "Remove Pages", "Delete selected pages while keeping the rest in order.", "file-minus", "#92400e", "remove_pdf_pages", "pdf-remove-pages"),
    pdf("pdf-extract-pages", "Extract PDF Pages", "Extract Pages", "Pull selected page ranges into a new PDF.", "file-search", "#6366f1", "extract_pdf_pages", "pdf-extract-pages"),
    { id: "pdf-organize", title: "Organize PDF", shortTitle: "Organize", blurb: "Reorder PDF pages and save an organized copy.", symbol: "layout-grid", tint: "#06b6d4", category: "PDF", command: "organize_pdf", verification: "pdf-organize", view: "pdf-organize", status: "implemented" },
    pdf("pdf-compress", "Compress PDF", "Compress", "Shrink scan-heavy PDFs by rasterizing pages as JPEG.", "file-download", "#10b981", "compress_pdf", "pdf-compress"),
    { id: "heic-convert", title: "Convert Image Format", shortTitle: "Convert", blurb: "Convert HEIC to PNG, JPEG, WebP, and back.", symbol: "arrows-exchange", tint: "#3b82f6", category: "Images", command: "convert_images", verification: "convert-image-format", view: "image-convert", status: "implemented" },
    { id: "compress", title: "Compress Images", shortTitle: "Compress", blurb: "Shrink image files losslessly or trade quality for size.", symbol: "file-download", tint: "#22c55e", category: "Images", command: "compress_images", verification: "compress-images", view: "image-compress", status: "implemented" },
    image("resize", "Resize Images", "Resize", "Scale images by pixels, percentage, or longest side.", "resize", "#a855f7", "resize_images", "resize-images"),
    image("rotate", "Rotate and Flip Images", "Rotate", "Rotate and mirror images without resampling.", "rotate", "#ec4899", "rotate_images", "rotate-images"),
    image("crop", "Crop Images", "Crop", "Crop images to an aspect ratio or pixel rectangle.", "crop", "#6366f1", "crop_images", "crop-images"),
    image("icon-set", "Generate App Icons", "Icons", "Turn one image into a complete app icon set.", "icons", "#f97316", "generate_icon_set", "icon-set"),
    image("gif-create", "Create GIF", "GIF Maker", "Animate a batch of still images into a looped GIF.", "movie", "#14b8a6", "create_gif", "gif-create"),
    image("gif-extract", "Extract GIF Frames", "Frames", "Split an animated GIF into individual frames.", "stack-2", "#14b8a6", "extract_gif_frames", "gif-extract"),
    image("image-watermark", "Watermark Images", "Watermark", "Stamp text or a logo across a batch of images.", "droplet-half", "#06b6d4", "watermark_images", "image-watermark"),
    image("image-metadata", "Image Metadata", "Metadata", "Inspect or strip EXIF and GPS data without recompression.", "info-circle", "#10b981", "image_metadata", "image-metadata"),
    image("image-tone", "Colour and Tone Adjustments", "Tone", "Adjust brightness, contrast, saturation, and exposure in batches.", "adjustments", "#eab308", "adjust_image_tone", "image-tone"),
    image("tiff-pages", "Split and Combine TIFF", "TIFF Pages", "Split a multi-page TIFF or combine images into one.", "files", "#92400e", "process_tiff_pages", "tiff-pages"),
    image("image-blur-faces", "Blur Faces", "Blur Faces", "Detect faces on-device and blur them in photos.", "face-id", "#ef4444", "blur_faces", "image-blur-faces"),
    image("image-remove-bg", "Remove Background", "Cutout", "Lift the subject out of a photo into a transparent PNG.", "wand", "#a855f7", "remove_image_background", "image-remove-bg"),
];

export const utilitiesByCategory = (category: ToolDefinition["category"]) =>
    UtilityRegistry.filter((utility) => utility.category === category);
