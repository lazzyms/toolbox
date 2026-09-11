import type {
    ToolDefinition,
    ToolWorkspaceDefinition,
    WorkspaceId,
} from "../contracts";
import { capabilityFor } from "./capabilities";

const UtilityMetadata: Array<Omit<ToolDefinition, "capability" | "status">> = [
    { id: "pdf-unlock", title: "Remove Password", shortTitle: "Remove Password", blurb: "Save an unlocked copy of a PDF, Word, Excel, or PowerPoint file.", symbol: "lock-open", tint: "#f97316", category: "Documents", command: "remove_password", verification: "remove-password", view: "pdf-unlock" },
    { id: "pdf-page-numbers", title: "Add PDF Page Numbers", shortTitle: "Page Numbers", blurb: "Stamp page numbers onto a PDF.", symbol: "numbers", tint: "#3b82f6", category: "PDF", command: "add_page_numbers", verification: "pdf-page-numbers", view: "pdf-page-numbers" },
    { id: "pdf-merge", title: "Merge PDF", shortTitle: "Merge", blurb: "Combine several PDFs into one file.", symbol: "files", tint: "#a855f7", category: "PDF", command: "merge_pdfs", verification: "pdf-merge", view: "pdf-merge" },
    { id: "pdf-watermark", title: "Watermark PDF", shortTitle: "Watermark", blurb: "Stamp text across PDF pages.", symbol: "droplet", tint: "#06b6d4", category: "PDF", command: "watermark_pdf", verification: "pdf-watermark", view: "pdf-watermark" },
    { id: "pdf-crop", title: "Crop PDF", shortTitle: "Crop", blurb: "Hide content outside a selected page rectangle.", symbol: "crop", tint: "#10b981", category: "PDF", command: "crop_pdf", verification: "pdf-crop", view: "pdf-crop" },
    { id: "pdf-edit", title: "Edit PDF", shortTitle: "Edit", blurb: "Add text, highlights, shapes, and notes to a PDF.", symbol: "wand", tint: "#8b5cf6", category: "PDF", command: "edit_pdf", verification: "pdf-edit", view: "pdf-edit" },
    { id: "pdf-protect", title: "Protect PDF", shortTitle: "Protect", blurb: "Add a password so only you can open a PDF.", symbol: "lock", tint: "#ef4444", category: "PDF", command: "protect_pdf", verification: "protect-pdf", view: "pdf-protect" },
    { id: "images-to-pdf", title: "Images to PDF", shortTitle: "Images to PDF", blurb: "Turn photos and scans into one PDF.", symbol: "photo", tint: "#3b82f6", category: "PDF", command: "images_to_pdf", verification: "images-to-pdf", view: "images-to-pdf" },
    { id: "pdf-to-images", title: "PDF to Images", shortTitle: "PDF to Images", blurb: "Render PDF pages to JPEG images at 72 to 300 dpi.", symbol: "photo", tint: "#6366f1", category: "PDF", command: "pdf_to_images", verification: "pdf-to-images", view: "pdf-to-images" },
    { id: "pdf-to-text", title: "PDF to Text", shortTitle: "PDF to Text", blurb: "Extract selectable PDF text into a text file.", symbol: "file-text", tint: "#6b7280", category: "PDF", command: "pdf_to_text", verification: "pdf-to-text", view: "pdf-to-text" },
    { id: "pdf-split", title: "Split PDF", shortTitle: "Split", blurb: "Break a PDF into separate page files.", symbol: "file-scissors", tint: "#f97316", category: "PDF", command: "split_pdf", verification: "pdf-split", view: "pdf-split" },
    { id: "pdf-image-extract", title: "Extract Images from PDF", shortTitle: "Extract Images", blurb: "Pull embedded JPEG pictures out at their original resolution.", symbol: "photo-search", tint: "#22c55e", category: "PDF", command: "extract_pdf_images", verification: "pdf-image-extract", view: "pdf-image-extract" },
    { id: "pdf-sign", title: "Sign PDF", shortTitle: "Sign", blurb: "Place a visible typed signature on a PDF page.", symbol: "signature", tint: "#ec4899", category: "PDF", command: "sign_pdf", verification: "pdf-sign", view: "pdf-sign" },
    { id: "pdf-ocr", title: "OCR PDF", shortTitle: "OCR", blurb: "Read text out of scans through an offline OCR adapter.", symbol: "scan", tint: "#14b8a6", category: "PDF", command: "ocr_pdf", verification: "pdf-ocr", view: "pdf-ocr" },
    { id: "pdf-remove-pages", title: "Remove PDF Pages", shortTitle: "Remove Pages", blurb: "Delete selected pages while keeping the rest in order.", symbol: "file-minus", tint: "#92400e", category: "PDF", command: "remove_pdf_pages", verification: "pdf-remove-pages", view: "pdf-remove-pages" },
    { id: "pdf-extract-pages", title: "Extract PDF Pages", shortTitle: "Extract Pages", blurb: "Pull selected page ranges into a new PDF.", symbol: "file-search", tint: "#6366f1", category: "PDF", command: "extract_pdf_pages", verification: "pdf-extract-pages", view: "pdf-extract-pages" },
    { id: "pdf-organize", title: "Organize PDF", shortTitle: "Organize", blurb: "Reorder, rotate, remove, or add blank PDF pages and save an organized copy.", symbol: "layout-grid", tint: "#06b6d4", category: "PDF", command: "organize_pdf", verification: "pdf-organize", view: "pdf-organize" },
    { id: "pdf-compress", title: "Compress PDF", shortTitle: "Compress", blurb: "Shrink PDF stream data without changing page geometry.", symbol: "file-download", tint: "#10b981", category: "PDF", command: "compress_pdf", verification: "pdf-compress", view: "pdf-compress" },
    { id: "heic-convert", title: "Convert Image Format", shortTitle: "Convert", blurb: "Convert HEIC to PNG, JPEG, WebP, and back.", symbol: "arrows-exchange", tint: "#3b82f6", category: "Images", command: "convert_images", verification: "convert-image-format", view: "image-convert" },
    { id: "compress", title: "Compress Images", shortTitle: "Compress", blurb: "Shrink image files losslessly or trade quality for size.", symbol: "file-download", tint: "#22c55e", category: "Images", command: "compress_images", verification: "compress-images", view: "image-compress" },
    { id: "resize", title: "Resize Images", shortTitle: "Resize", blurb: "Scale images to exact dimensions.", symbol: "resize", tint: "#a855f7", category: "Images", command: "resize_images", verification: "resize-images", view: "image-resize" },
    { id: "rotate", title: "Rotate and Flip Images", shortTitle: "Rotate", blurb: "Rotate images by a right angle.", symbol: "rotate", tint: "#ec4899", category: "Images", command: "rotate_images", verification: "rotate-images", view: "image-rotate" },
    { id: "crop", title: "Crop Images", shortTitle: "Crop", blurb: "Crop images to a pixel rectangle.", symbol: "crop", tint: "#6366f1", category: "Images", command: "crop_images", verification: "crop-images", view: "image-crop" },
    { id: "icon-set", title: "Generate App Icons", shortTitle: "Icons", blurb: "Turn one image into a complete app icon set.", symbol: "icons", tint: "#f97316", category: "Images", command: "generate_icon_set", verification: "icon-set", view: "image-icons" },
    { id: "gif-create", title: "Create GIF", shortTitle: "GIF Maker", blurb: "Animate a batch of still images into a looped GIF.", symbol: "movie", tint: "#14b8a6", category: "Images", command: "create_gif", verification: "gif-create", view: "gif-create" },
    { id: "gif-extract", title: "Extract GIF Frames", shortTitle: "Frames", blurb: "Split an animated GIF into individual frames.", symbol: "stack-2", tint: "#14b8a6", category: "Images", command: "extract_gif_frames", verification: "gif-extract", view: "gif-extract" },
    { id: "image-watermark", title: "Watermark Images", shortTitle: "Watermark", blurb: "Apply a visible watermark effect across images.", symbol: "droplet-half", tint: "#06b6d4", category: "Images", command: "watermark_images", verification: "image-watermark", view: "image-watermark" },
    { id: "image-metadata", title: "Image Metadata", shortTitle: "Metadata", blurb: "Save a copy without image metadata.", symbol: "info-circle", tint: "#10b981", category: "Images", command: "image_metadata", verification: "image-metadata", view: "image-metadata" },
    { id: "image-tone", title: "Colour and Tone Adjustments", shortTitle: "Tone", blurb: "Adjust image brightness and contrast in batches.", symbol: "adjustments", tint: "#eab308", category: "Images", command: "adjust_image_tone", verification: "image-tone", view: "image-tone" },
    { id: "tiff-pages", title: "Split and Combine TIFF", shortTitle: "TIFF Pages", blurb: "Process TIFF images without changing the source file.", symbol: "files", tint: "#92400e", category: "Images", command: "process_tiff_pages", verification: "tiff-pages", view: "tiff-pages" },
    { id: "image-blur-faces", title: "Blur Faces", shortTitle: "Blur Faces", blurb: "Detect and blur faces through an offline vision adapter.", symbol: "face-id", tint: "#ef4444", category: "Images", command: "blur_faces", verification: "image-blur-faces", view: "image-blur-faces" },
    { id: "image-remove-bg", title: "Remove Background", shortTitle: "Cutout", blurb: "Remove backgrounds through an offline vision adapter.", symbol: "wand", tint: "#a855f7", category: "Images", command: "remove_image_background", verification: "image-remove-bg", view: "image-remove-bg" },
];

export const UtilityRegistry: ToolDefinition[] = UtilityMetadata.map((utility) => {
    const capability = capabilityFor(utility.command);
    return {
        ...utility,
        status: capability.nativeAvailability === "unavailable" ? "unavailable" : "implemented",
        capability,
    };
});

export const utilitiesByCategory = (category: ToolDefinition["category"]) =>
    UtilityRegistry.filter((utility) => utility.category === category);

/**
 * Workspaces are the user-facing layer over the atomic command registry.
 * Keep the atomic IDs stable: verification, saved navigation, and native
 * command contracts still address individual outcomes.
 */
export const ToolWorkspaceRegistry: ToolWorkspaceDefinition[] = [
    {
        id: "file-security",
        title: "Protect & unlock files",
        blurb: "Open a file once, then protect it or remove its password without leaving the workspace.",
        symbol: "lock",
        tint: "#ef4444",
        category: "Documents",
        categories: ["Documents", "PDF"],
        toolIds: ["pdf-unlock", "pdf-protect"],
    },
    {
        id: "pdf-editor",
        title: "PDF editor",
        blurb: "Open a PDF and work on its content, pages, overlays, and final output from one editor surface.",
        symbol: "edit-pdf",
        tint: "#8b5cf6",
        category: "PDF",
        categories: ["PDF"],
        toolIds: [
            "pdf-edit",
            "pdf-crop",
            "pdf-watermark",
            "pdf-sign",
            "pdf-page-numbers",
            "pdf-remove-pages",
            "pdf-organize",
        ],
    },
    {
        id: "pdf-convert",
        title: "PDF conversion",
        blurb: "Move between PDF pages, images, and selectable text with one input surface and clear outputs.",
        symbol: "arrows-exchange",
        tint: "#6366f1",
        category: "PDF",
        categories: ["PDF"],
        toolIds: [
            "pdf-to-images",
            "pdf-to-text",
            "pdf-image-extract",
            "images-to-pdf",
            "pdf-ocr",
            "pdf-merge",
            "pdf-split",
            "pdf-extract-pages",
            "pdf-compress",
        ],
    },
    {
        id: "image-editor",
        title: "Image editor",
        blurb: "Tune, crop, resize, rotate, flip, watermark, compress, or convert images in one photo-style workspace.",
        symbol: "adjustments",
        tint: "#eab308",
        category: "Images",
        categories: ["Images"],
        toolIds: ["heic-convert", "compress", "resize", "rotate", "crop", "image-watermark", "image-tone"],
    },
    {
        id: "media-tools",
        title: "Media utilities",
        blurb: "Generate icons, work with GIF and TIFF sequences, inspect metadata, and access optional vision tools.",
        symbol: "layers",
        tint: "#14b8a6",
        category: "Images",
        categories: ["Images", "PDF"],
        toolIds: [
            "icon-set",
            "gif-create",
            "gif-extract",
            "tiff-pages",
            "image-metadata",
            "image-blur-faces",
            "image-remove-bg",
        ],
    },
];

export const toolsForWorkspace = (workspace: ToolWorkspaceDefinition) =>
    workspace.toolIds
        .map((id) => UtilityRegistry.find((utility) => utility.id === id))
        .filter(Boolean) as ToolDefinition[];

export const workspaceById = (workspaceId: WorkspaceId) =>
    ToolWorkspaceRegistry.find((workspace) => workspace.id === workspaceId);

export const toolsForWorkspaceId = (workspaceId: WorkspaceId) => {
    const workspace = workspaceById(workspaceId);
    return workspace ? toolsForWorkspace(workspace) : [];
};

export const workspaceForTool = (toolId: string) =>
    ToolWorkspaceRegistry.find((workspace) => workspace.toolIds.includes(toolId as ToolDefinition["id"]));
