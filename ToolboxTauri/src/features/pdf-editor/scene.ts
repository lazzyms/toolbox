import type { WatermarkOptions, WatermarkPosition } from '../../contracts';
import type { PdfDocument, PdfTextRun } from './contracts';

// All geometry is in points, from the top-left of the unrotated visible source page.
export interface SceneRect { x: number; y: number; width: number; height: number }
export interface ScenePoint { x: number; y: number }
export type SceneTool = 'select' | 'text' | 'highlight' | 'shape' | 'signature' | 'watermark' | 'crop';
export type SceneShape = 'square' | 'round' | 'triangle' | 'line' | 'dotted-line' | 'arrow-left' | 'arrow-right' | 'arrow-up' | 'arrow-down';
export type SignatureMode = 'image' | 'text';
export type WatermarkLayer = 'front' | 'behind';
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
  // Text boxes and watermarks: a page list like "1-3, 7, 10-" or "all", "odd", "even".
  // The object is stored once on its page and drawn on every page in the list.
  pageRange?: string | null;
  watermark?: WatermarkOptions;
  layer?: WatermarkLayer;
  // Picture watermarks: a local preview URL and the picture's pixel size, for the page preview only.
  watermarkPreview?: string | null;
  watermarkImageSize?: { width: number; height: number } | null;
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
}
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
  // Watermarks drawn on this page, wherever they are stored; null removes one.
  watermarks?: SceneObject[];
  onWatermarkChange?: (id: string, patch: Partial<SceneObject> | null) => void;
}
export const sceneFromDocument = (document: PdfDocument): PdfScene => ({
  pages: document.pages.map((page) => ({ id: `source-${page.index}`, sourceIndex: page.index,
    width: page.width, height: page.height, rotation: 0, crop: null,
    sourceRotation: page.rotation ?? 0,
    sourceBox: page.pageBox ? { x: page.pageBox[0], y: page.pageBox[1], width: page.pageBox[2] - page.pageBox[0], height: page.pageBox[3] - page.pageBox[1] } : null,
    objects: [] })),
});
export const visibleBounds = (page: ScenePage): SceneRect => page.crop ?? { x: 0, y: 0, width: page.width, height: page.height };

export const pageRangeError = (range: string) => `“${range}” is not a page list. Use page numbers and ranges like 1-3, 7, 10-, or all, odd or even.`;

// Mirrors page_range_pages in the native exporter: zero-based pages, sorted, with ranges clamped to the document.
export function pagesInRange(range: string, pageCount: number): number[] {
  const invalid = () => new Error(pageRangeError(range));
  const number = (value: string) => {
    const text = value.trim();
    if (!/^\d+$/.test(text) || Number(text) < 1) throw invalid();
    return Number(text);
  };
  const keywords: Record<string, [number, number]> = { all: [1, 1], odd: [1, 2], even: [2, 2] };
  const pages = new Set<number>();
  for (const raw of range.split(',')) {
    const item = raw.trim();
    if (!item) throw invalid();
    const keyword = keywords[item.toLowerCase()];
    if (keyword) {
      for (let page = keyword[0]; page <= pageCount; page += keyword[1]) pages.add(page - 1);
      continue;
    }
    const dash = item.indexOf('-');
    const first = number(dash < 0 ? item : item.slice(0, dash));
    const last = dash < 0 ? first : item.slice(dash + 1).trim() === '' ? Infinity : number(item.slice(dash + 1));
    if (first > last) throw invalid();
    for (let page = first; page <= Math.min(last, pageCount); page += 1) pages.add(page - 1);
  }
  if (!pages.size) throw new Error(`Pages ${range} are not in this document. It has ${pageCount} page${pageCount === 1 ? '' : 's'}.`);
  return [...pages].sort((a, b) => a - b);
}

// Objects drawn on a page: its own, plus range-applied objects stored on other pages.
export function objectsOnPage(scene: PdfScene, pageIndex: number): { object: SceneObject; origin: number }[] {
  return scene.pages.flatMap((page, origin) => page.objects.flatMap((object) => {
    if (object.pageRange == null) return origin === pageIndex ? [{ object, origin }] : [];
    try { return pagesInRange(object.pageRange, scene.pages.length).includes(pageIndex) ? [{ object, origin }] : []; } catch { return []; }
  }));
}

// The five fixed patterns of the earlier watermark tool, as shared options.
export const legacyWatermarkPatterns = {
  'across-page': { position: 'tile', rotation: -35 },
  'bottom-right-to-top-left': { position: 'center', rotation: -45 },
  'top-right-to-bottom-left': { position: 'center', rotation: 45 },
  'center-horizontal': { position: 'center', rotation: 0 },
  'center-vertical': { position: 'center', rotation: 90 },
} as const satisfies Record<string, Pick<WatermarkOptions, 'position' | 'rotation'>>;

export const WATERMARK_DEFAULT_SIZE = 0.1;
const WATERMARK_LINE_HEIGHT = 1.2;
const WATERMARK_MAX_TILES = 2000;

// Mirrors watermark_size and watermark_centers in the native exporter. Draft text width is estimated;
// the rendered preview replaces the draft once it is ready.
export function watermarkMarks(area: SceneRect, object: SceneObject): { width: number; height: number; rotation: number; centers: ScenePoint[] } {
  const options = object.watermark;
  const shorter = Math.min(area.width, area.height);
  let width = 0, height = 0;
  if (options?.text?.trim()) {
    const fontSize = (options.size ?? WATERMARK_DEFAULT_SIZE) * shorter;
    const lines = options.text.split('\n');
    width = Math.max(...lines.map((line) => line.length)) * fontSize * 0.6;
    height = lines.length * WATERMARK_LINE_HEIGHT * fontSize;
  } else if (options?.logoPath && object.watermarkImageSize) {
    const { width: w, height: h } = object.watermarkImageSize;
    const scale = options.size !== undefined ? options.size * shorter / h : Math.min(180 / w, 100 / h, 1);
    width = w * scale; height = h * scale;
  }
  const rotation = options?.rotation ?? 0;
  const radians = rotation * Math.PI / 180;
  const boxW = width * Math.abs(Math.cos(radians)) + height * Math.abs(Math.sin(radians));
  const boxH = width * Math.abs(Math.sin(radians)) + height * Math.abs(Math.cos(radians));
  const margin = 0.04 * shorter;
  const left = area.x + margin + boxW / 2, top = area.y + margin + boxH / 2;
  const right = area.x + area.width - margin - boxW / 2, bottom = area.y + area.height - margin - boxH / 2;
  const cx = area.x + area.width / 2, cy = area.y + area.height / 2;
  const position: WatermarkPosition = options?.position ?? 'center';
  let centers: ScenePoint[];
  if (typeof position === 'object') centers = [{ x: area.x + position.x + boxW / 2, y: area.y + position.y + boxH / 2 }];
  else if (position === 'tile') {
    const gap = Math.max(height, 1), stepX = boxW + gap, stepY = boxH + gap;
    const columns = Math.ceil(area.width / 2 / stepX) + 1, rows = Math.ceil(area.height / 2 / stepY) + 1;
    centers = [];
    if ((2 * columns + 1) * (2 * rows + 1) <= WATERMARK_MAX_TILES) {
      for (let row = -rows; row <= rows; row += 1) for (let column = -columns; column <= columns; column += 1) {
        centers.push({ x: cx + column * stepX + (Math.abs(row) % 2 === 1 ? stepX / 2 : 0), y: cy + row * stepY });
      }
    }
  } else centers = [{ x: { center: cx, 'top-left': left, 'bottom-left': left, 'top-right': right, 'bottom-right': right }[position],
    y: { center: cy, 'top-left': top, 'top-right': top, 'bottom-left': bottom, 'bottom-right': bottom }[position] }];
  return { width, height, rotation, centers };
}

// The selectable box of a watermark on a page: its first mark, or the centre tile when tiled.
export function watermarkBounds(area: SceneRect, object: SceneObject): SceneRect {
  const { width, height, rotation, centers } = watermarkMarks(area, object);
  const radians = rotation * Math.PI / 180;
  const boxW = Math.max(1, width * Math.abs(Math.cos(radians)) + height * Math.abs(Math.sin(radians)));
  const boxH = Math.max(1, width * Math.abs(Math.sin(radians)) + height * Math.abs(Math.cos(radians)));
  const centre = object.watermark?.position === 'tile' ? { x: area.x + area.width / 2, y: area.y + area.height / 2 } : centers[0] ?? { x: area.x + area.width / 2, y: area.y + area.height / 2 };
  return { x: centre.x - boxW / 2, y: centre.y - boxH / 2, width: boxW, height: boxH };
}
