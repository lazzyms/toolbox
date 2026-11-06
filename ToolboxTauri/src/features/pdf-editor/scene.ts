import type { PdfDocument, PdfEditRun, PdfEditRunReason, PdfFormField, PdfTextEdit, PdfTextRun } from './contracts';

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
  // Text boxes and signatures: a page list like "1-3, 7, 10-"; null keeps the object on its own page.
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
export interface SceneRedaction { id: string; rect: SceneRect; color: string }
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
  formValue?: (field: PdfFormField) => string | boolean;
  onFormValue?: (field: PdfFormField, value: string | boolean) => void;
  redactionFill?: string;
  redactionPreview?: boolean;
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

// Page-sequence edits. Each takes a scene and page ids and returns the next scene.
// Returns null when the selection covers every page, because a PDF keeps at least one.
// Text edits belong to source pages, so the edits on removed pages go with them.
export const removePages = (scene: PdfScene, ids: string[]): PdfScene | null => {
  const { textEdits, ...rest } = scene;
  const pages = scene.pages.filter((page) => !ids.includes(page.id));
  if (!pages.length) return null;
  const kept = textEdits?.filter((edit) => pages.some((page) => page.sourceIndex === edit.pageIndex));
  return { ...rest, pages, ...(kept?.length ? { textEdits: kept } : {}) };
};

// Moves the pages to the target's position, keeping their relative order.
export const movePagesBefore = (scene: PdfScene, ids: string[], targetId: string): PdfScene => {
  if (ids.includes(targetId) || !scene.pages.some((page) => page.id === targetId)) return scene;
  const moving = scene.pages.filter((page) => ids.includes(page.id));
  const pages = scene.pages.filter((page) => !ids.includes(page.id));
  pages.splice(pages.findIndex((page) => page.id === targetId), 0, ...moving);
  return { ...scene, pages };
};

export const swapPage = (scene: PdfScene, index: number, delta: number): PdfScene => {
  const to = index + delta;
  if (index < 0 || index >= scene.pages.length || to < 0 || to >= scene.pages.length) return scene;
  const pages = [...scene.pages];
  [pages[index], pages[to]] = [pages[to], pages[index]];
  return { ...scene, pages };
};

export const rotatePages = (scene: PdfScene, ids: string[], degrees = 90): PdfScene => ({
  ...scene,
  pages: scene.pages.map((page) => ids.includes(page.id) ? { ...page, rotation: (((page.rotation + degrees) % 360) + 360) % 360 } : page),
});
