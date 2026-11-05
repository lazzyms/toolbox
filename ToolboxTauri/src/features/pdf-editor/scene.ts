import type { PdfDocument, PdfTextRun } from './contracts';

// All geometry is in points, from the top-left of the unrotated visible source page.
export interface SceneRect { x: number; y: number; width: number; height: number }
export interface ScenePoint { x: number; y: number }
export type SceneTool = 'select' | 'text' | 'highlight' | 'shape' | 'signature' | 'watermark' | 'crop' | 'redact';
export type SceneShape = 'square' | 'round' | 'triangle' | 'line' | 'dotted-line' | 'arrow-left' | 'arrow-right' | 'arrow-up' | 'arrow-down';
export type SignatureMode = 'image' | 'text';
export type WatermarkPattern = 'across-page' | 'bottom-right-to-top-left' | 'top-right-to-bottom-left' | 'center-horizontal' | 'center-vertical';
export type HighlightMode = 'area' | 'text-selection';
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
  watermarkPattern?: WatermarkPattern;
}
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
export interface PdfScene { pages: ScenePage[] }
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
  redactionFill?: string;
  redactionPreview?: boolean;
}
export const sceneFromDocument = (document: PdfDocument): PdfScene => ({
  pages: document.pages.map((page) => ({ id: `source-${page.index}`, sourceIndex: page.index,
    width: page.width, height: page.height, rotation: 0, crop: null,
    sourceRotation: page.rotation ?? 0,
    sourceBox: page.pageBox ? { x: page.pageBox[0], y: page.pageBox[1], width: page.pageBox[2] - page.pageBox[0], height: page.pageBox[3] - page.pageBox[1] } : null,
    objects: [] })),
});
export const visibleBounds = (page: ScenePage): SceneRect => page.crop ?? { x: 0, y: 0, width: page.width, height: page.height };
