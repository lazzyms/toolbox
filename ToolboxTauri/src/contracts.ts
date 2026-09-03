export type ToolCategory = "PDF" | "Images";

export type ToolStatus = "implemented" | "planned";

export type OutputLocation = "alongsideInput" | { customFolder: string };

export type JobState = "running" | "success" | "mixed" | "failure";

export interface JobOutcome {
    inputPath: string;
    outputPaths: string[];
    detail: string;
    failure: string | null;
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
    command: string;
    verification: string;
    view: "pdf-unlock" | "pdf-protect" | "pdf-crop" | "pdf-sign" | "pdf-organize" | "pdf-page-numbers" | "pdf-watermark" | "pdf-compress" | "pdf-remove-pages" | "pdf-extract-pages" | "pdf-merge" | "pdf-split" | "pdf-to-images" | "pdf-to-text" | "pdf-image-extract" | "images-to-pdf" | "image-compress" | "image-convert" | "planned";
}

export interface ToolRequest {
    paths: string[];
    outputLocation: OutputLocation;
}

export interface PDFRequest extends ToolRequest {
    password: string;
}

export interface CompressImagesRequest extends ToolRequest {
    quality: number;
}

export interface ConvertImagesRequest extends ToolRequest {
    format: "jpg" | "png" | "webp" | "heic";
}
