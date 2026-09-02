export type EditorLayout = "vertical" | "horizontal";

export type PageScope =
    | { kind: "all" }
    | { kind: "selected"; pages: number[] };

export interface PdfPage {
    index: number;
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
    scope: PageScope;
    layout: EditorLayout;
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
}

export interface OrganizePdfRequest {
    paths: string[];
    pageOrder: number[];
    scope: PdfPageScope;
    outputLocation: "alongsideInput";
}
