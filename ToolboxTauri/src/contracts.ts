export type ToolCategory = "PDF" | "Images" | "Documents";

export type ToolStatus = "implemented" | "planned";

export type OutputLocation = "alongsideInput" | { customFolder: string };

export type JobState = "running" | "success" | "mixed" | "failure";

export type ToolCommand =
  | "remove_password"
  | "protect_pdf"
  | "compress_images"
  | "convert_images"
  | "inspect_pdf"
  | "crop_pdf"
  | "edit_pdf"
  | "sign_pdf"
  | "organize_pdf"
  | "add_page_numbers"
  | "watermark_pdf"
  | "compress_pdf"
  | "remove_pdf_pages"
  | "extract_pdf_pages"
  | "merge_pdfs"
  | "split_pdf"
  | "pdf_to_images"
  | "pdf_to_text"
  | "extract_pdf_images"
  | "images_to_pdf"
  | "ocr_pdf"
  | "blur_faces"
  | "remove_image_background"
  | "resize_images"
  | "rotate_images"
  | "crop_images"
  | "adjust_image_tone"
  | "watermark_images"
  | "generate_icon_set"
  | "create_gif"
  | "extract_gif_frames"
  | "process_tiff_pages"
  | "image_metadata";

export type ErrorKind =
  "invalidInput" | "unavailable" | "limitExceeded" | "processing";

export interface ToolError {
  kind: ErrorKind;
  message: string;
}

export interface JobOutcome {
  inputPath: string;
  outputPaths: string[];
  detail: string;
  failure: ToolError | null;
}

export type ToolResult = JobOutcome[];

export interface Progress {
  completed: number;
  total: number;
}

export interface ToolDefinition {
  id: string;
  title: string;
  shortTitle: string;
  blurb: string;
  symbol: string;
  tint: string;
  category: ToolCategory;
  status: ToolStatus;
  command: ToolCommand;
  verification: string;
  view:
    | "pdf-unlock"
    | "pdf-protect"
    | "pdf-crop"
    | "pdf-edit"
    | "pdf-sign"
    | "pdf-organize"
    | "pdf-page-numbers"
    | "pdf-watermark"
    | "pdf-compress"
    | "pdf-remove-pages"
    | "pdf-extract-pages"
    | "pdf-merge"
    | "pdf-split"
    | "pdf-to-images"
    | "pdf-to-text"
    | "pdf-image-extract"
    | "images-to-pdf"
    | "pdf-ocr"
    | "image-blur-faces"
    | "image-remove-bg"
    | "image-resize"
    | "image-rotate"
    | "image-crop"
    | "image-compress"
    | "image-convert"
    | "image-watermark"
    | "image-tone"
    | "image-icons"
    | "gif-create"
    | "gif-extract"
    | "tiff-pages"
    | "image-metadata"
    | "planned";
}

export interface ToolRequest {
  paths: string[];
  outputLocation: OutputLocation;
}

export interface PDFRequest extends ToolRequest {
  password: string;
}

export interface PasswordRequest extends ToolRequest {
  password: string;
}

export interface CompressImagesRequest extends ToolRequest {
  quality: number;
  lossless?: boolean;
}

export interface ConvertImagesRequest extends ToolRequest {
  format: "jpg" | "png" | "webp" | "heic";
}
