import type { OutputLocation } from "../../contracts";
import type { PdfScene } from "./scene";
import type { RedactionMark } from "./subtools";

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
    editRuns?: PdfEditRun[] | null;
}

export interface PdfTextRun {
    text: string;
    x: number;
    y: number;
    width: number;
    height: number;
}

export type PdfEditRunReason = "picture" | "encoding" | "tagged" | "unsupported";

export interface PdfEditRun extends PdfTextRun {
    index: number;
    fontName: string;
    fontSize: number;
    reason: PdfEditRunReason | null;
}

export interface PdfTextEdit {
    pageIndex: number;
    run: number;
    original: string;
    replacement: string;
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
// `option` is the radio choice a widget stands for.
export interface PdfFormWidget { page: number; x: number; y: number; width: number; height: number; option?: string }
export type PdfFormFormat =
    | { kind: 'date'; pattern: string }
    | { kind: 'number'; decimals: number; separator: number; negative: number; currency: string; currencyPrepend: boolean }
    | { kind: 'percent'; decimals: number; separator: number }
    | { kind: 'special'; special: number };
// Text and choice fields hold strings, check boxes booleans, list boxes the chosen options.
export type PdfFormEntry = string | boolean | string[];
export interface PdfFormField {
    name: string;
    label: string | null;
    type: 'text' | 'checkbox' | 'radio' | 'dropdown' | 'listbox';
    value: PdfFormEntry;
    // Export values; `optionLabels` holds what the PDF shows for each one.
    options: string[];
    optionLabels: string[];
    flags: number;
    required: boolean;
    readOnly: boolean;
    multiline: boolean;
    multiSelect: boolean;
    editable: boolean;
    maxLen: number | null;
    format: PdfFormFormat | null;
    widgets: PdfFormWidget[];
}
// Fields arrive in Tab order. `unsupported` explains why a form can't be filled at all.
export interface PdfFormInspection { fields: PdfFormField[]; unsupported: string | null; comments: number }
export type PdfFlattenChoice = 'forms' | 'annotations' | 'both';
export interface PdfFormValue { name: string; value: PdfFormEntry }

export interface PdfSceneExportRequest {
    paths: string[];
    scene: PdfScene;
    outputLocation: OutputLocation;
    /** Export As: the chosen file. Never replaced; a taken name is numbered. */
    outputPath?: string;
    formValues?: PdfFormValue[];
    textEdits?: PdfTextEdit[];
    flatten?: PdfFlattenChoice;
    redactions?: RedactionMark[];
}
