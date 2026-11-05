import type { PdfDocument, PdfEditRun, PdfEditRunReason, PdfFormEntry, PdfFormField, PdfTextEdit, PdfTextRun } from './contracts';

// All geometry is in points, from the top-left of the unrotated visible source page.
export interface SceneRect { x: number; y: number; width: number; height: number }
export interface ScenePoint { x: number; y: number }
export type SceneTool = 'select' | 'edit-text' | 'text' | 'highlight' | 'shape' | 'signature' | 'form' | 'watermark' | 'crop' | 'redact';
export type SceneShape = 'square' | 'round' | 'triangle' | 'line' | 'dotted-line' | 'arrow-left' | 'arrow-right' | 'arrow-up' | 'arrow-down';
export type SignatureMode = 'image' | 'text';
export type WatermarkPattern = 'across-page' | 'bottom-right-to-top-left' | 'top-right-to-bottom-left' | 'center-horizontal' | 'center-vertical';
export type HighlightMode = 'area' | 'text-selection';
export type TextFontFamily = 'Noto Sans' | 'Noto Serif' | 'Noto Sans Mono';
export type TextAlign = 'left' | 'center' | 'right';
export interface SceneObject {
  id: string;
  kind: 'text' | 'highlight' | 'shape' | 'signature' | 'watermark';
  rect: SceneRect;
  text: string;
  fontSize: number;
  color: string;
  opacity: number;
  // Signature points are normalized to the object's rectangle (0..1).
  strokes: ScenePoint[][];
  shape?: SceneShape;
  highlightMode?: HighlightMode;
  signatureMode?: SignatureMode;
  signaturePath?: string | null;
  // This is a local asset URL used only by the page preview. The native
  // exporter uses signaturePath, keeping the file processing on-device.
  signaturePreview?: string | null;
  fontFamily?: string;
  bold?: boolean;
  italic?: boolean;
  align?: TextAlign;
  // Text boxes only: a page list like "1-3, 7, 10-"; null keeps the box on its own page.
  pageRange?: string | null;
  watermarkPattern?: WatermarkPattern;
}
// Defaults for the next text box; each new text box copies them.
export interface TextStyle { fontFamily: TextFontFamily; bold: boolean; italic: boolean; align: TextAlign }
export interface ScenePage {
  id: string;
  sourceIndex: number | null;
  width: number;
  height: number;
  rotation: number;
  crop: SceneRect | null;
  sourceRotation: number | null;
  sourceBox: SceneRect | null;
  objects: SceneObject[];
  // Marks are drawn only on the canvas; export sends them as the request's redactions section.
  redactions?: SceneRedaction[];
}
/** `label` names what a text mark covers, such as “Asha Patel”. It is shown in the marks list only and never exported. */
export interface SceneRedaction { id: string; rect: SceneRect; color: string; label?: string }

/** Joins word boxes that share a line into one box per line, keeping reading order. */
export const textLines = (runs: { text: string; x: number; y: number; width: number; height: number }[]) =>
  runs.reduce<{ rect: SceneRect; text: string }[]>((lines, run) => {
    const last = lines[lines.length - 1];
    if (last && Math.abs(last.rect.y - run.y) < Math.min(last.rect.height, run.height) / 2 && run.x >= last.rect.x) {
      const top = Math.min(last.rect.y, run.y), bottom = Math.max(last.rect.y + last.rect.height, run.y + run.height);
      last.rect = { x: last.rect.x, y: top, width: Math.max(last.rect.x + last.rect.width, run.x + run.width) - last.rect.x, height: bottom - top };
      last.text += ` ${run.text}`;
    } else lines.push({ rect: { x: run.x, y: run.y, width: run.width, height: run.height }, text: run.text });
    return lines;
  }, []);
export interface PdfScene { pages: ScenePage[]; textEdits?: PdfTextEdit[] }
export interface ScenePreview { dataUrl: string; width: number; height: number }
export interface SceneCanvasProps {
  page: ScenePage;
  sourcePreview: string | null;
  textRuns: PdfTextRun[];
  renderedPreview: ScenePreview | null;
  tool: SceneTool;
  selectedId: string | null;
  zoom: number;
  onSelect: (id: string | null) => void;
  onCommit: (page: ScenePage) => void;
  makeObject: (kind: SceneObject['kind'], rect: SceneRect) => SceneObject;
  // Form fields on this page, in Tab order; shown only while filling the form.
  formFields?: PdfFormField[];
  formValue?: (field: PdfFormField) => PdfFormEntry;
  onFormValue?: (field: PdfFormField, value: PdfFormEntry) => void;
  // The flag shown under a field the user left empty or invalid, such as "Required".
  formFlag?: (field: PdfFormField) => string | null;
  onFormFocus?: (field: PdfFormField) => void;
  onFormLeave?: (field: PdfFormField) => void;
  redactionFill?: string;
  redactionPreview?: boolean;
  /** Text search matches on this page, drawn while the results are under review. */
  searchMatches?: SceneRect[];
  editRuns: PdfEditRun[];
  textEdits: PdfTextEdit[];
  selectedRun: number | null;
  onSelectRun: (run: number | null) => void;
  onEditRun: (run: PdfEditRun, replacement: string) => void;
}
export const sceneFromDocument = (document: PdfDocument): PdfScene => ({
  pages: document.pages.map((page) => ({ id: `source-${page.index}`, sourceIndex: page.index,
    width: page.width, height: page.height, rotation: 0, crop: null,
    sourceRotation: page.rotation ?? 0,
    sourceBox: page.pageBox ? { x: page.pageBox[0], y: page.pageBox[1], width: page.pageBox[2] - page.pageBox[0], height: page.pageBox[3] - page.pageBox[1] } : null,
    objects: [] })),
});
export const visibleBounds = (page: ScenePage): SceneRect => page.crop ?? { x: 0, y: 0, width: page.width, height: page.height };
// Words of the replacement that end past the original run's width, assuming the
// original font's average character width.
export const textOverflowWords = (original: string, replacement: string): number => {
  let end = 0;
  return replacement.split(/(\s+)/).filter((part) => {
    end += part.length;
    return part.trim() !== '' && end > original.length;
  }).length;
};
export const notEditableReasons: Record<PdfEditRunReason, { short: string; detail: string }> = {
  picture: { short: 'This text is part of a picture, so it can\'t be edited.',
    detail: 'It is part of a scanned picture, not real text. Text drawn in a font with no character map is treated the same way.' },
  encoding: { short: 'This text uses a font with no character map, so it can\'t be edited.',
    detail: 'Its font has no character map, so its letters can\'t be changed safely. It is treated like text in a picture.' },
  tagged: { short: 'This text is linked to accessibility tags, so it can\'t be edited.',
    detail: 'It is linked to the PDF\'s accessibility tags, and changing it would break them.' },
  unsupported: { short: 'This text can\'t be rewritten safely, so it can\'t be edited.',
    detail: 'This page\'s content can\'t be rewritten safely.' },
};
export const overflowMessage = (words: number) =>
  `Text runs past the original line by ${words} ${words === 1 ? 'word' : 'words'}. It will be cut off unless you make it shorter or smaller.`;
