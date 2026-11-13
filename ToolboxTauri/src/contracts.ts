export type ToolCategory = "PDF" | "Images" | "Documents";

export type ToolStatus = "implemented" | "planned" | "unavailable";

export type AtomicToolId =
  | "lock-unlock"
  | "pdf-page-numbers"
  | "pdf-merge"
  | "pdf-watermark"
  | "pdf-crop"
  | "pdf-edit"
  | "images-to-pdf"
  | "pdf-to-images"
  | "pdf-extract"
  | "convert-to-pdf"
  | "pdf-split"
  | "pdf-sign"
  | "pdf-ocr"
  | "pdf-remove-pages"
  | "pdf-extract-pages"
  | "pdf-organize"
  | "compress-files"
  | "pdf-rotate"
  | "pdf-repair"
  | "pdf-redact"
  | "pdf-compare"
  | "pdf-privacy"
  | "pdf-properties"
  | "pdf-bates"
  | "heic-convert"
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
  | "compress-files"
  | "media-tools";

/** "none" marks a creation tool: it takes no input files and accepts no extensions. */
export type ToolInputCardinality = "single" | "multiple" | "ordered" | "none";

export type NativeAvailability = "available" | "unavailable";

export interface FormatFamily {
  id: string;
  extensions: readonly string[];
  nativeAvailability: NativeAvailability;
  basicFidelity: boolean;
}

export interface ToolCapability {
  acceptedExtensions: readonly string[];
  formatFamilies?: readonly FormatFamily[];
  /** Extensions the command takes only to report each file Skipped. The open dialog never offers them. */
  skippedExtensions: readonly string[];
  inputCardinality: ToolInputCardinality;
  supportsPageSelection: boolean;
  supportsPreview: boolean;
  nativeAvailability: NativeAvailability;
  outputSuffix: string;
  manyToOne: boolean;
  outputFormats?: readonly string[];
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
  | "protect_files"
  | "convert_images"
  | "inspect_pdf"
  | "crop_pdf"
  | "resize_pdf"
  | "export_pdf_scene"
  | "bates_number_pdfs"
  | "compress_files"
  | "rotate_pdf"
  | "repair_pdf"
  | "inspect_pdf_privacy"
  | "edit_pdf_properties"
  | "redact_pdf"
  | "compare_pdfs"
  | "extract_pdf_pages"
  | "merge_pdfs"
  | "split_pdf"
  | "pdf_to_images"
  | "extract_from_pdf"
  | "images_to_pdf"
  | "convert_to_pdf"
  | "ocr_pdf"
  | "blur_faces"
  | "remove_image_background"
  | "export_image_edit_stack"
  | "generate_icon_set"
  | "create_gif"
  | "extract_gif_frames"
  | "process_tiff_pages";


export type OutcomePointer =
  | { kind: "tool"; toolId: AtomicToolId }
  | { kind: "pack"; packId: string; name: string; reason: PackNeed }
  | { kind: "outputFolder" };

/** Why a file needs its pack: `PackNeed` in `src-tauri/src/kit/contracts.rs`. */
export type PackNeed = "notInstalled" | "damaged" | "incompatible";

export type LimitKind = "fileSize" | "pageCount" | "time" | "decompression" | "textLength" | "dimensions";

export type ToolError = { message: string; pointer: OutcomePointer | null } & (
  | { kind: "invalidInput" | "unavailable" | "processing" | "wrongPassword" | "cancelled" }
  | { kind: "limitExceeded"; details: { limit: LimitKind } }
);

export type ErrorKind = ToolError["kind"];

export type SkipCode =
  | "notProtected"
  | "alreadyProtected"
  | "noSavings"
  | "alreadyInTargetFormat"
  | "rangeEmptyAfterClamping"
  | "textAlreadyPresent"
  | "noEffectiveChange"
  | "unsupportedFormat"
  | "needsOcr"
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
  | { code: "precisionReduced"; message: string }
  | { code: "bookmarksNotCarried"; message: string; details: { into: string } }
  | { code: "tagsNotCarried"; message: string; details: { into: string } }
  | { code: "omittedContent"; message: string }
  | { code: "approximateLayout"; message: string }
  | { code: "imageUnsupported"; message: string; details: { page: number; image: number; feature: string } }
  | { code: "imageDamaged"; message: string; details: { page: number; image: number } }
  | { code: "imageTooLarge"; message: string; details: { page: number; image: number; width: number; height: number } }
  | { code: "jpegColourKept"; message: string; details: { colour: JpegColour; pages: number[]; images: number } }
  | { code: "pagesWithoutText"; message: string; details: { pages: number[] } }
  | { code: "transparencyFlattened"; message: string };

export const jpegColours = ["cmyk", "inverted"] as const;
export type JpegColour = (typeof jpegColours)[number];

export type WarningCode = OutcomeWarning["code"];

export type FontStyle = "regular" | "bold" | "italic" | "boldItalic";

export type FontSource = "embedded" | "system" | "bundled fallback" | "bundled default";

export interface FontUse {
  requestedFamily: string;
  requestedStyle: FontStyle;
  fontUsed: string;
  styleUsed: FontStyle;
  source: FontSource;
}

/** One labelled line of a result's breakdown, such as which pages were recognized. */
export interface OutcomeDetail {
  label: string;
  value: string;
}

interface JobOutcomeBase {
  inputPath: string;
  detail: string;
  warnings: OutcomeWarning[];
  fonts?: FontUse[];
  originalBytes?: number;
  outputBytes?: number;
  pageDetails?: OutcomeDetail[];
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

export type ImageMetadataFormat = "jpeg" | "png" | "webp" | "tiff" | "heic";

export type ImageMetadataErrorCode = "unreadable" | "unsupportedFormat" | "truncated" | "corrupt";

export type ImageMetadataInspection =
  | {
      status: "report";
      path: string;
      format: ImageMetadataFormat;
      exif: boolean;
      gps: boolean;
      xmp: boolean;
      iptc: boolean;
      comments: boolean;
      icc: boolean;
      orientation: number | null;
    }
  | { status: "error"; path: string; code: ImageMetadataErrorCode; message: string };

export interface ToolDefinition {
  id: AtomicToolId;
  title: string;
  shortTitle: string;
  resultVerb: string;
  resultNoun?: string;
  runningVerb: string;
  runningDetail?: string;
  outputNoun?: string;
  lockedNote?: string;
  /** The tool's own words for a result row whose copy lost its digital signature, when its board has them. */
  signatureRemovedNote?: string;
  blurb: string;
  keywords?: readonly string[];
  symbol: string;
  category: ToolCategory;
  status: ToolStatus;
  capability: ToolCapability;
  command: ToolCommand;
  verification: string;
  view:
    | "lock-unlock"
    | "pdf-crop"
    | "pdf-edit"
    | "pdf-sign"
    | "pdf-organize"
    | "pdf-page-numbers"
    | "pdf-watermark"
    | "compress-files"
    | "pdf-rotate"
    | "pdf-repair"
    | "pdf-redact"
    | "pdf-compare"
    | "pdf-privacy"
    | "pdf-properties"
    | "pdf-bates"
    | "pdf-remove-pages"
    | "pdf-extract-pages"
    | "pdf-merge"
    | "pdf-split"
    | "pdf-to-images"
    | "pdf-extract"
    | "images-to-pdf"
    | "convert-to-pdf"
    | "pdf-ocr"
    | "image-blur-faces"
    | "image-remove-bg"
    | "image-resize"
    | "image-rotate"
    | "image-crop"
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
  /** Every tool in the view, in list order. Each tool belongs to exactly one view. */
  toolIds: AtomicToolId[];
  /** Tools shown as one entry whose tabs or modes are those tools. The entry opens the first one. */
  editor?: { title: string; blurb: string; symbol: string; toolIds: AtomicToolId[] };
  /** Labelled groups listed after the main tools, in this order, such as Details and More. */
  groups?: { label: string; toolIds: AtomicToolId[] }[];
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

/** Low keeps the most detail and High saves the most space. Every PDF is repacked without loss; Medium and High also recompress and downsample its photographs. JPEGs are re-saved at quality 85, 75 or 50. PNGs are repacked without loss at Low and Medium and cut to a dithered 256-colour palette at High. Lossless WebPs stay lossless at Low and Medium and become lossy at High; lossy WebPs are re-saved at the JPEG qualities. */
export type CompressLevel = "low" | "medium" | "high";

export interface CompressFilesRequest extends ToolRequest {
  level: CompressLevel;
  stripExif: boolean;
}

export type WatermarkPlacement = "center" | "top-left" | "top-right" | "bottom-left" | "bottom-right" | "tile";
/** A placement, a legacy pen offset in pixels, or an anchor from 0 to 1 that lands the mark in the same relative place on every size. */
export type WatermarkPosition = WatermarkPlacement | { x: number; y: number } | { anchorX: number; anchorY: number };

// Mirrors kit::common::watermark::WatermarkOptions.
export interface WatermarkOptions {
  text?: string;
  logoPath?: string;
  size?: number;
  opacity: number;
  color?: string;
  position?: WatermarkPosition;
  rotation?: number;
}

export type OcrMode = "searchablePdf" | "text";

export interface OcrRequest extends ToolRequest {
  pageRange: string | null;
  mode: OcrMode;
  force: boolean;
  languages: string[];
}

export type TiffCompression = "lzw" | "none" | "deflate";

export interface ConvertImagesRequest extends ToolRequest {
  format: "jpg" | "png" | "webp" | "heic" | "tiff";
  /** Sent only with a TIFF target. */
  tiffCompression?: TiffCompression;
}
