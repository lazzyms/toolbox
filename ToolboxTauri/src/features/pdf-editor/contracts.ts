import type { OutputLocation } from "../../contracts";
import type { PdfScene } from "./scene";

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
    outputLocation: OutputLocation;
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
    outputLocation: OutputLocation;
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
    outputLocation: OutputLocation;
}

// Widget rectangles use scene coordinates: points from the top-left of the unrotated source page.
export interface PdfFormWidget { page: number; x: number; y: number; width: number; height: number }
export interface PdfFormField {
    name: string;
    label: string | null;
    type: 'text' | 'checkbox';
    value: string | boolean;
    options: string[];
    flags: number;
    required: boolean;
    readOnly: boolean;
    multiline: boolean;
    maxLen: number | null;
    widgets: PdfFormWidget[];
}
// Fields arrive in Tab order. `unsupported` explains why a form can't be filled at all.
export interface PdfFormInspection { fields: PdfFormField[]; unsupported: string | null }
export interface PdfFormValue { name: string; value: string | boolean }

export interface PdfSceneExportRequest {
    paths: string[];
    scene: PdfScene;
    outputLocation: OutputLocation;
    formValues?: PdfFormValue[];
    textEdits?: unknown[];
    flatten?: boolean;
    redactions?: unknown[];
}
