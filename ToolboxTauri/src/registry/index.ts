import type {
    AtomicToolId,
    ToolDefinition,
    ToolWorkspaceDefinition,
    WorkspaceId,
} from "../contracts";
import { capabilityFor } from "./capabilities";

const UtilityMetadata: Array<Omit<ToolDefinition, "capability" | "status">> = [
    { id: "lock-unlock", title: "Lock & Unlock", shortTitle: "Lock & Unlock", resultVerb: "Unlocked", runningVerb: "Unlocking", runningDetail: "Writing and checking the copy", lockedNote: "Mode, passwords and files are locked until the batch finishes.", blurb: "Remove a password you know from PDF, Word, Excel and PowerPoint files.", keywords: ["lock", "unlock", "protect", "password", "encrypt", "decrypt"], symbol: "lock-open", category: "Documents", command: "remove_password", verification: "lock-unlock", view: "lock-unlock" },
    { id: "pdf-page-numbers", title: "Add PDF Page Numbers", shortTitle: "Page Numbers", resultVerb: "Numbered", runningVerb: "Numbering", blurb: "Stamp page numbers onto a PDF.", symbol: "numbers", category: "PDF", command: "add_page_numbers", verification: "pdf-page-numbers", view: "pdf-page-numbers" },
    { id: "pdf-merge", title: "Merge PDF", shortTitle: "Merge", resultVerb: "Merged", runningVerb: "Merging", blurb: "Combine several PDFs into one file.", symbol: "files", category: "PDF", command: "merge_pdfs", verification: "pdf-merge", view: "pdf-merge" },
    { id: "pdf-watermark", title: "Watermark PDF", shortTitle: "Watermark", resultVerb: "Watermarked", runningVerb: "Watermarking", blurb: "Stamp text across PDF pages.", symbol: "droplet", category: "PDF", command: "watermark_pdf", verification: "pdf-watermark", view: "pdf-watermark" },
    { id: "pdf-crop", title: "Crop PDF", shortTitle: "Crop", resultVerb: "Cropped", runningVerb: "Cropping", blurb: "Hide content outside a selected page rectangle.", symbol: "crop", category: "PDF", command: "crop_pdf", verification: "pdf-crop", view: "pdf-crop" },
    { id: "pdf-edit", title: "Edit PDF", shortTitle: "Edit", resultVerb: "Edited", runningVerb: "Editing", blurb: "Add text, highlights, shapes, and notes to a PDF.", symbol: "wand", category: "PDF", command: "edit_pdf", verification: "pdf-edit", view: "pdf-edit" },
    { id: "images-to-pdf", title: "Images to PDF", shortTitle: "Images to PDF", resultVerb: "Converted", runningVerb: "Converting", blurb: "Turn photos and scans into one PDF.", symbol: "photo", category: "PDF", command: "images_to_pdf", verification: "images-to-pdf", view: "images-to-pdf" },
    { id: "pdf-to-images", title: "PDF to Images", shortTitle: "PDF to Images", resultVerb: "Converted", runningVerb: "Converting", blurb: "Render PDF pages to JPEG images at 72 to 300 dpi.", symbol: "photo", category: "PDF", command: "pdf_to_images", verification: "pdf-to-images", view: "pdf-to-images" },
    { id: "pdf-to-text", title: "PDF to Text", shortTitle: "PDF to Text", resultVerb: "Extracted", runningVerb: "Extracting", blurb: "Extract selectable PDF text into a text file.", symbol: "file-text", category: "PDF", command: "pdf_to_text", verification: "pdf-to-text", view: "pdf-to-text" },
    { id: "pdf-split", title: "Split PDF", shortTitle: "Split", resultVerb: "Split", runningVerb: "Splitting", blurb: "Break a PDF into separate page files.", symbol: "file-scissors", category: "PDF", command: "split_pdf", verification: "pdf-split", view: "pdf-split" },
    { id: "pdf-image-extract", title: "Extract Images from PDF", shortTitle: "Extract Images", resultVerb: "Extracted", runningVerb: "Extracting", blurb: "Pull embedded JPEG pictures out at their original resolution.", symbol: "photo-search", category: "PDF", command: "extract_pdf_images", verification: "pdf-image-extract", view: "pdf-image-extract" },
    { id: "pdf-sign", title: "Sign PDF", shortTitle: "Sign", resultVerb: "Signed", runningVerb: "Signing", blurb: "Place a visible typed signature on a PDF page.", symbol: "signature", category: "PDF", command: "sign_pdf", verification: "pdf-sign", view: "pdf-sign" },
    { id: "pdf-ocr", title: "OCR PDF", shortTitle: "OCR", resultVerb: "Recognised", runningVerb: "Recognising", blurb: "Read text out of scans through an offline OCR adapter.", symbol: "scan", category: "PDF", command: "ocr_pdf", verification: "pdf-ocr", view: "pdf-ocr" },
    { id: "pdf-remove-pages", title: "Remove PDF Pages", shortTitle: "Remove Pages", resultVerb: "Trimmed", runningVerb: "Trimming", blurb: "Delete selected pages while keeping the rest in order.", symbol: "file-minus", category: "PDF", command: "remove_pdf_pages", verification: "pdf-remove-pages", view: "pdf-remove-pages" },
    { id: "pdf-extract-pages", title: "Extract PDF Pages", shortTitle: "Extract Pages", resultVerb: "Extracted", runningVerb: "Extracting", blurb: "Pull selected page ranges into a new PDF.", symbol: "file-search", category: "PDF", command: "extract_pdf_pages", verification: "pdf-extract-pages", view: "pdf-extract-pages" },
    { id: "pdf-organize", title: "Organize PDF", shortTitle: "Organize", resultVerb: "Organized", runningVerb: "Organizing", blurb: "Reorder, rotate, remove, or add blank PDF pages and save an organized copy.", symbol: "layout-grid", category: "PDF", command: "organize_pdf", verification: "pdf-organize", view: "pdf-organize" },
    { id: "pdf-compress", title: "Compress PDF", shortTitle: "Compress", resultVerb: "Compressed", runningVerb: "Compressing", blurb: "Shrink PDF stream data without changing page geometry.", symbol: "file-download", category: "PDF", command: "compress_pdf", verification: "pdf-compress", view: "pdf-compress" },
    { id: "pdf-rotate", title: "Rotate PDFs", shortTitle: "Rotate", resultVerb: "Rotated", runningVerb: "Rotating", blurb: "Turn all pages, or just the ones you pick.", symbol: "rotate", category: "PDF", command: "rotate_pdf", verification: "pdf-rotate", view: "pdf-rotate" },
    { id: "pdf-repair", title: "Repair PDF", shortTitle: "Repair", resultVerb: "Repaired", runningVerb: "Repairing", blurb: "Fix a damaged PDF that won't open properly.", symbol: "tool", category: "PDF", command: "repair_pdf", verification: "pdf-repair", view: "pdf-repair" },
    { id: "pdf-redact", title: "Redact PDF", shortTitle: "Redact", resultVerb: "Redacted", runningVerb: "Redacting", blurb: "Black out private details so they can't be recovered.", symbol: "eye-off", category: "PDF", command: "redact_pdf", verification: "pdf-redact", view: "pdf-redact" },
    { id: "pdf-compare", title: "Compare PDFs", shortTitle: "Compare", resultVerb: "Compared", runningVerb: "Comparing", blurb: "See what text changed between two versions of a PDF.", symbol: "git-compare", category: "PDF", command: "compare_pdfs", verification: "pdf-compare", view: "pdf-compare" },
    { id: "heic-convert", title: "Convert Image Format", shortTitle: "Convert", resultVerb: "Converted", runningVerb: "Converting", blurb: "Convert HEIC to PNG, JPEG, WebP, and back.", symbol: "arrows-exchange", category: "Images", command: "convert_images", verification: "convert-image-format", view: "image-convert" },
    { id: "compress", title: "Compress Images", shortTitle: "Compress", resultVerb: "Compressed", runningVerb: "Compressing", blurb: "Shrink image files losslessly or trade quality for size.", symbol: "file-download", category: "Images", command: "compress_images", verification: "compress-images", view: "image-compress" },
    { id: "resize", title: "Resize Images", shortTitle: "Resize", resultVerb: "Resized", runningVerb: "Resizing", blurb: "Scale images to exact dimensions.", symbol: "resize", category: "Images", command: "resize_images", verification: "resize-images", view: "image-resize" },
    { id: "rotate", title: "Rotate and Flip Images", shortTitle: "Rotate", resultVerb: "Rotated", runningVerb: "Rotating", blurb: "Rotate images by a right angle.", symbol: "rotate", category: "Images", command: "rotate_images", verification: "rotate-images", view: "image-rotate" },
    { id: "crop", title: "Crop Images", shortTitle: "Crop", resultVerb: "Cropped", runningVerb: "Cropping", blurb: "Crop images to a pixel rectangle.", symbol: "crop", category: "Images", command: "crop_images", verification: "crop-images", view: "image-crop" },
    { id: "icon-set", title: "Generate App Icons", shortTitle: "Icons", resultVerb: "Generated", runningVerb: "Generating", blurb: "Turn one image into a complete app icon set.", symbol: "icons", category: "Images", command: "generate_icon_set", verification: "icon-set", view: "image-icons" },
    { id: "gif-create", title: "Create GIF", shortTitle: "GIF Maker", resultVerb: "Created", runningVerb: "Creating", blurb: "Animate a batch of still images into a looped GIF.", symbol: "movie", category: "Images", command: "create_gif", verification: "gif-create", view: "gif-create" },
    { id: "gif-extract", title: "Extract GIF Frames", shortTitle: "Frames", resultVerb: "Extracted", runningVerb: "Extracting", blurb: "Split an animated GIF into individual frames.", symbol: "stack-2", category: "Images", command: "extract_gif_frames", verification: "gif-extract", view: "gif-extract" },
    { id: "image-watermark", title: "Watermark Images", shortTitle: "Watermark", resultVerb: "Watermarked", runningVerb: "Watermarking", blurb: "Apply a visible watermark effect across images.", symbol: "droplet-half", category: "Images", command: "watermark_images", verification: "image-watermark", view: "image-watermark" },
    { id: "image-metadata", title: "Image Metadata", shortTitle: "Metadata", resultVerb: "Cleaned", runningVerb: "Cleaning", blurb: "Save a copy without image metadata.", symbol: "info-circle", category: "Images", command: "image_metadata", verification: "image-metadata", view: "image-metadata" },
    { id: "image-tone", title: "Colour and Tone Adjustments", shortTitle: "Tone", resultVerb: "Adjusted", runningVerb: "Adjusting", blurb: "Adjust image brightness and contrast in batches.", symbol: "adjustments", category: "Images", command: "adjust_image_tone", verification: "image-tone", view: "image-tone" },
    { id: "tiff-pages", title: "Split and Combine TIFF", shortTitle: "TIFF Pages", resultVerb: "Processed", runningVerb: "Processing", blurb: "Process TIFF images without changing the source file.", symbol: "files", category: "Images", command: "process_tiff_pages", verification: "tiff-pages", view: "tiff-pages" },
    { id: "image-blur-faces", title: "Blur Faces", shortTitle: "Blur Faces", resultVerb: "Blurred", runningVerb: "Blurring", blurb: "Detect and blur faces through an offline vision adapter.", symbol: "face-id", category: "Images", command: "blur_faces", verification: "image-blur-faces", view: "image-blur-faces" },
    { id: "image-remove-bg", title: "Remove Background", shortTitle: "Cutout", resultVerb: "Cut out", runningVerb: "Cutting out", blurb: "Remove backgrounds through an offline vision adapter.", symbol: "wand", category: "Images", command: "remove_image_background", verification: "image-remove-bg", view: "image-remove-bg" },
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
        title: "File Security",
        blurb: "Open a file once, then protect it or remove its password without leaving the workspace.",
        symbol: "lock",
        category: "Documents",
        categories: ["Documents", "PDF"],
        toolIds: ["lock-unlock"],
    },
    {
        id: "pdf-editor",
        title: "PDF Editor",
        blurb: "Open a PDF and work on its content and pages, or merge, split and fix whole files.",
        symbol: "edit-pdf",
        category: "PDF",
        categories: ["PDF"],
        toolIds: [
            "pdf-edit",
            "pdf-watermark",
            "pdf-sign",
            "pdf-remove-pages",
            "pdf-organize",
            "pdf-merge",
            "pdf-split",
            "pdf-crop",
            "pdf-page-numbers",
            "pdf-redact",
            "pdf-rotate",
            "pdf-repair",
            "pdf-compare",
            "pdf-extract-pages",
        ],
        editor: {
            title: "Edit PDF",
            blurb: "Add text, highlights, shapes, signatures and watermarks, or delete pages.",
            symbol: "edit-pdf",
            toolIds: ["pdf-edit", "pdf-watermark", "pdf-sign", "pdf-remove-pages"],
        },
        moreToolIds: ["pdf-rotate", "pdf-repair", "pdf-compare", "pdf-extract-pages"],
    },
    {
        id: "pdf-convert",
        title: "PDF Conversion",
        blurb: "Move between PDF pages, images, and selectable text with one input surface and clear outputs.",
        symbol: "arrows-exchange",
        category: "PDF",
        categories: ["PDF"],
        toolIds: ["pdf-to-images", "images-to-pdf", "pdf-ocr", "pdf-to-text", "pdf-image-extract"],
    },
    {
        id: "image-editor",
        title: "Image Editor",
        blurb: "Resize, crop, rotate, adjust or watermark many images at once, convert them, or remove their metadata.",
        symbol: "image-tone",
        category: "Images",
        categories: ["Images"],
        toolIds: ["resize", "crop", "rotate", "image-tone", "image-watermark", "heic-convert", "image-metadata"],
        editor: {
            title: "Edit Image",
            blurb: "Resize, crop, rotate, adjust or watermark many images at once.",
            symbol: "image-tone",
            toolIds: ["resize", "crop", "rotate", "image-tone", "image-watermark"],
        },
    },
    {
        id: "compress-files",
        title: "Compress Files",
        blurb: "Make PDFs and images smaller.",
        symbol: "compress-pdf",
        category: "PDF",
        categories: ["PDF", "Images"],
        toolIds: ["pdf-compress", "compress"],
    },
    {
        id: "media-tools",
        title: "Media",
        blurb: "Generate icons, work with GIF and TIFF sequences, and access optional vision tools.",
        symbol: "layers",
        category: "Images",
        categories: ["Images", "PDF"],
        toolIds: [
            "icon-set",
            "gif-create",
            "gif-extract",
            "tiff-pages",
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

/** One line in a view's tool list: a single tool, or the view's editor with its tools as tabs. */
export interface WorkspaceEntry {
    key: string;
    title: string;
    blurb: string;
    symbol: string;
    /** The tools this entry stands for. Opening the entry opens the first one. */
    tools: ToolDefinition[];
    more: boolean;
}

/**
 * List entries for a view, built from the tools the caller can open, in list order with More last.
 * `groupEditor: false` lists editor tools by their own names, for search results and favorites.
 */
export const workspaceEntries = (
    workspace: ToolWorkspaceDefinition,
    tools: readonly ToolDefinition[],
    { groupEditor = true }: { groupEditor?: boolean } = {},
): WorkspaceEntry[] => {
    const editorTools = tools.filter((tool) => workspace.editor?.toolIds.includes(tool.id));
    const entries = tools.flatMap((tool): WorkspaceEntry[] => {
        const more = workspace.moreToolIds?.includes(tool.id) ?? false;
        if (!groupEditor || !workspace.editor?.toolIds.includes(tool.id)) {
            return [{ key: tool.id, title: tool.title, blurb: tool.blurb, symbol: tool.symbol, tools: [tool], more }];
        }
        if (tool !== editorTools[0]) return [];
        const { title, blurb, symbol } = workspace.editor;
        return [{ key: `${workspace.id}-editor`, title, blurb, symbol, tools: editorTools, more }];
    });
    return [...entries.filter((entry) => !entry.more), ...entries.filter((entry) => entry.more)];
};

/** PDF Editor tools that open the full-window page editor rather than a batch screen. */
const pdfSceneToolIds: readonly AtomicToolId[] = ["pdf-edit", "pdf-crop", "pdf-watermark", "pdf-sign", "pdf-page-numbers", "pdf-remove-pages", "pdf-organize"];
export const isPdfSceneTool = (id: AtomicToolId) => pdfSceneToolIds.includes(id);
