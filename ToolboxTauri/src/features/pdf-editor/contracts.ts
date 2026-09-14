export type EditorLayout = "vertical" | "horizontal";
export type EditorDensity = "compact" | "comfortable";

export type PageScope =
    | { kind: "all" }
    | { kind: "selected"; pages: number[] };

export interface PdfPage {
    index: number;
    x?: number;
    y?: number;
    width: number;
    height: number;
    rotation?: number;
    pageBox?: [number, number, number, number];
    preview?: string | null;
    textRuns?: PdfTextRun[] | null;
}

export interface PdfTextRun {
    text: string;
    x: number;
    y: number;
    width: number;
    height: number;
}

export interface PdfDocument {
    path: string;
    pages: PdfPage[];
}

export interface PdfEditorState {
    currentPage: number;
    selectedPages: number[];
    pageOrder: number[];
    deletedPages: number[];
    rotatePages: { page: number; degrees: number }[];
    scope: PageScope;
    layout: EditorLayout;
    density: EditorDensity;
    error: string | null;
}

export interface PreviewSize {
    width: number;
    height: number;
}

export interface PdfRect {
    x: number;
    y: number;
    width: number;
    height: number;
}

export type PdfPageScope = "all" | { selected: { pages: number[] } };

export interface CropPdfRequest {
    paths: string[];
    rectangle: PdfRect;
    scope: PdfPageScope;
    outputLocation: "alongsideInput";
}

export interface SignPdfRequest extends CropPdfRequest {
    page: number;
    text: string;
    signaturePath: string | null;
}

export interface OrganizePdfRequest {
    paths: string[];
    pageOrder: number[];
    deletePages: number[];
    rotatePages: { page: number; degrees: number }[];
    scope: PdfPageScope;
    outputLocation: "alongsideInput";
}

export type PdfOverlay =
    | { kind: "edit"; mode: "text" | "note" | "highlight" | "shape"; text: string; pages: number[] | null; rectangle: PdfRect }
    | { kind: "watermark"; text: string; opacity: number; position: string | null; logoPath: string | null; pages: number[] | null }
    | { kind: "sign"; page: number; text: string; signaturePath: string | null; rectangle: PdfRect; scope: PdfPageScope }
    | { kind: "pageNumbers"; startNumber: number | null; fontSize: number | null; position: string | null; pages: number[] | null };

export type PdfEditOperation =
    | { kind: "crop"; rectangle: PdfRect; scope: PdfPageScope }
    | { kind: "overlay"; overlay: PdfOverlay }
    | { kind: "addPages"; page: number; position: "before" | "after" | "end"; count: number };

export interface PdfEditSessionPlan {
    pageOrder: number[];
    deletePages: number[];
    rotatePages: { page: number; degrees: number }[];
    operations: PdfEditOperation[];
}

export interface PdfEditSessionRequest {
    paths: string[];
    plan: PdfEditSessionPlan;
    outputLocation: "alongsideInput";
}
