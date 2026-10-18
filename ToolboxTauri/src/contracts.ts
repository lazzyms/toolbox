export type ToolCategory = "PDF" | "Images" | "Documents";

export type ToolStatus = "implemented" | "planned" | "unavailable";

export type AtomicToolId =
  | "pdf-unlock"
  | "pdf-page-numbers"
  | "pdf-merge"
  | "pdf-watermark"
  | "pdf-crop"
  | "pdf-edit"
  | "pdf-protect"
  | "office-protect"
  | "images-to-pdf"
  | "pdf-to-images"
  | "pdf-to-text"
  | "pdf-split"
  | "pdf-image-extract"
  | "pdf-sign"
  | "pdf-ocr"
  | "pdf-remove-pages"
  | "pdf-extract-pages"
  | "pdf-organize"
  | "pdf-compress"
  | "heic-convert"
  | "compress"
  | "resize"
  | "rotate"
  | "crop"
  | "icon-set"
  | "gif-create"
  | "gif-extract"
  | "image-watermark"
  | "image-metadata"
  | "image-tone"
  | "tiff-pages"
  | "image-blur-faces"
  | "image-remove-bg";

export type WorkspaceId =
  | "file-security"
  | "pdf-editor"
  | "pdf-convert"
  | "image-editor"
  | "media-tools";

export type ToolInputCardinality = "single" | "multiple" | "ordered";

export type NativeAvailability = "available" | "unavailable";

export interface ToolCapability {
  acceptedExtensions: readonly string[];
  inputCardinality: ToolInputCardinality;
  supportsPageSelection: boolean;
  supportsPreview: boolean;
  nativeAvailability: NativeAvailability;
  outputSuffix: string;
  manyToOne: boolean;
}

export type OutputLocation = "alongsideInput" | { customFolder: string };

export type OutputMode = "alongsideInput" | "customFolder";

export interface OutputSettings {
  mode: OutputMode;
  folder: string;
}

export type JobState = "running" | "success" | "mixed" | "failure";

export type ToolCommand =
  | "remove_password"
  | "protect_pdf"
  | "protect_office"
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


export type OutcomePointer =
  | { kind: "tool"; toolId: AtomicToolId }
  | { kind: "pack"; packId: string }
  | { kind: "outputFolder" };

export type LimitKind = "fileSize" | "pageCount" | "time" | "decompression";

export type ToolError = { message: string; pointer: OutcomePointer | null } & (
  | { kind: "invalidInput" | "unavailable" | "processing" | "wrongPassword" | "cancelled" }
  | { kind: "limitExceeded"; details: { limit: LimitKind } }
);

export type ErrorKind = ToolError["kind"];

export type SkipCode =
  | "notProtected"
  | "alreadyProtected"
  | "permissionsPasswordOnly"
  | "noSavings"
  | "alreadyInTargetFormat"
  | "rangeEmptyAfterClamping"
  | "textAlreadyPresent"
  | "noEffectiveChange"
  | "unsupportedFormat"
  | "cancelled";

export interface SkipReason {
  code: SkipCode;
  message: string;
  pointer: OutcomePointer | null;
}

export type OutcomeWarning =
  | { code: "signatureRemoved"; message: string }
  | { code: "fontSubstituted"; message: string; details: { requestedFamily: string; actualFamily: string } }
  | { code: "renamedToAvoidReplacing"; message: string; details: { chosenName: string; finalName: string } }
  | { code: "precisionReduced"; message: string };

export type WarningCode = OutcomeWarning["code"];

interface JobOutcomeBase {
  inputPath: string;
  detail: string;
  warnings: OutcomeWarning[];
}

export interface SucceededJobOutcome extends JobOutcomeBase {
  state: "succeeded";
  outputPaths: string[];
  failure: null;
  reason: null;
}

export interface SkippedJobOutcome extends JobOutcomeBase {
  state: "skipped";
  outputPaths: [];
  failure: null;
  reason: SkipReason;
}

export interface FailedJobOutcome extends JobOutcomeBase {
  state: "failed";
  outputPaths: string[];
  failure: ToolError;
  reason: null;
}

export type JobOutcome = SucceededJobOutcome | SkippedJobOutcome | FailedJobOutcome;

export type ToolResult = JobOutcome[];

export interface Progress {
  completed: number;
  total: number;
}

export interface ImagePreview {
  width: number;
  height: number;
  dataUrl: string;
}

export interface ToolDefinition {
  id: AtomicToolId;
  title: string;
  shortTitle: string;
  resultVerb: string;
  runningVerb: string;
  blurb: string;
  symbol: string;
  category: ToolCategory;
  status: ToolStatus;
  capability: ToolCapability;
  command: ToolCommand;
  verification: string;
  view:
    | "pdf-unlock"
    | "pdf-protect"
    | "pdf-crop"
    | "pdf-edit"
    | "pdf-sign"
    | "office-protect"
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

export interface ToolWorkspaceDefinition {
  id: WorkspaceId;
  title: string;
  blurb: string;
  symbol: string;
  category: ToolCategory;
  categories: ToolCategory[];
  toolIds: AtomicToolId[];
}

export interface ToolRequest {
  paths: string[];
  outputLocation: OutputLocation;
}

// One password for the whole batch plus per-file overrides keyed by input path.
// It lives only in view memory: never in settings, storage, logs or analytics.
export interface PasswordSet {
  batch: string;
  overrides: Readonly<Record<string, string>>;
}

// The native side uses a file's override first and the batch password otherwise.
export interface PasswordRequest extends ToolRequest {
  password: string;
  passwordOverrides: Record<string, string>;
}

export interface CompressImagesRequest extends ToolRequest {
  quality: number;
  lossless?: boolean;
}

export interface ConvertImagesRequest extends ToolRequest {
  format: "jpg" | "png" | "webp" | "heic";
}
