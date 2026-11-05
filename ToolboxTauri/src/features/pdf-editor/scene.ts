import type { WatermarkOptions, WatermarkPosition } from '../../contracts';
import type { PdfDocument, PdfTextRun } from './contracts';

// All geometry is in points, from the top-left of the unrotated visible source page.
export interface SceneRect { x: number; y: number; width: number; height: number }
export interface ScenePoint { x: number; y: number }
export type SceneTool = 'select' | 'text' | 'highlight' | 'shape' | 'signature' | 'watermark' | 'pageNumbers' | 'headerFooter' | 'crop';
export type SceneShape = 'square' | 'round' | 'triangle' | 'line' | 'dotted-line' | 'arrow-left' | 'arrow-right' | 'arrow-up' | 'arrow-down';
export type SignatureMode = 'image' | 'text';
export type WatermarkLayer = 'front' | 'behind';
export type HighlightMode = 'area' | 'text-selection';
export type TextFontFamily = 'Noto Sans' | 'Noto Serif' | 'Noto Sans Mono';
export type TextAlign = 'left' | 'center' | 'right';
export type PageNumberFormat = 'number' | 'page' | 'page-of-total' | 'number-of-total' | 'dashes';
export type PageNumberPosition = `${'top' | 'middle' | 'bottom'}-${'left' | 'center' | 'right'}`;
export interface PageNumberOptions { format: PageNumberFormat; start: number; position: PageNumberPosition; margin: number }
export type HeaderFooterBand = 'header' | 'footer';
export type HeaderFooterSlot = 'left' | 'center' | 'right';
export type HeaderFooterText = Record<HeaderFooterBand, Record<HeaderFooterSlot, string>>;
// Slot text is a template with {page}, {pages}, {date} and {file} tags. The date is a fixed yyyy-mm-dd,
// and the file name is captured when the header and footer are added, so exports are reproducible.
export interface HeaderFooterOptions extends HeaderFooterText {
  firstPage: HeaderFooterText | null;
  even: HeaderFooterText | null;
  date: string;
  fileName: string;
  margin: number;
}
export interface HeaderFooterPart { band: HeaderFooterBand; slot: HeaderFooterSlot; text: string; rect: SceneRect }
export interface SceneObject {
  id: string;
  kind: 'text' | 'highlight' | 'shape' | 'signature' | 'watermark' | 'pageNumbers' | 'headerFooter';
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
  // Text boxes, watermarks, page numbers and headers and footers: a page list like "1-3, 7, 10-" or "all", "odd", "even".
  // The object is stored once on its page and drawn on every page in the list.
  pageRange?: string | null;
  watermark?: WatermarkOptions;
  layer?: WatermarkLayer;
  // Picture watermarks: a local preview URL and the picture's pixel size, for the page preview only.
  watermarkPreview?: string | null;
  watermarkImageSize?: { width: number; height: number } | null;
  pageNumbers?: PageNumberOptions;
  headerFooter?: HeaderFooterOptions;
  // The resolved header and footer text drawn on one page, for the page preview only.
  headerFooterParts?: HeaderFooterPart[];
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
  // Range-applied objects drawn on this page, wherever they are stored; null removes one.
  rangedObjects?: SceneObject[];
  onRangedChange?: (id: string, patch: Partial<SceneObject> | null) => void;
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

// Range-applied kinds are stored once and drawn on every page in their range, and outlive the page that stores them.
export const isRangedKind = (kind: SceneObject['kind']) => kind === 'watermark' || kind === 'pageNumbers' || kind === 'headerFooter';

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

export const pageNumberTemplates: Record<PageNumberFormat, string> = {
  number: '{page}', page: 'Page {page}', 'page-of-total': 'Page {page} of {pages}',
  'number-of-total': '{page} / {pages}', dashes: '- {page} -',
};

export interface PageTokens { page: number; pages: number; date?: string; file?: string }
const months = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];
// Mirrors token_date in the native exporter: 2026-10-13 shows as 13 October 2026.
export function tokenDate(iso: string): string | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(iso);
  if (!match) return null;
  const [year, month, day] = match.slice(1).map(Number);
  const date = new Date(Date.UTC(year, month - 1, day));
  if (date.getUTCFullYear() !== year || date.getUTCMonth() !== month - 1 || date.getUTCDate() !== day) return null;
  return `${day} ${months[month - 1]} ${year}`;
}
// Mirrors page_tokens in the native exporter. Each tag is replaced once, so tag-like text in a file name stays as it is.
export const resolvePageTokens = (template: string, tokens: PageTokens) =>
  template.replace(/\{(page|pages|date|file)\}/g, (tag, name: keyof PageTokens) => {
    if (name === 'date') return tokens.date === undefined ? tag : tokenDate(tokens.date) ?? '';
    const value = tokens[name];
    return value === undefined ? tag : String(value);
  });

// Numbers follow the final page order: the page at pageIndex shows start + pageIndex.
export function pageNumberText(object: SceneObject, pageIndex: number, pageCount: number): string {
  const options = object.pageNumbers;
  if (!options) return '';
  return resolvePageTokens(pageNumberTemplates[options.format], { page: options.start + pageIndex, pages: options.start + pageCount - 1 });
}

// Mirrors the page-number placement in the native exporter. Draft text width is estimated.
export function pageNumberRect(area: SceneRect, object: SceneObject, text: string): SceneRect {
  const width = Math.max(1, text.length * object.fontSize * 0.55), height = object.fontSize * 1.2;
  const [vertical, horizontal] = (object.pageNumbers?.position ?? 'bottom-center').split('-');
  const margin = object.pageNumbers?.margin ?? 0;
  const place = (start: number, size: number, length: number, side: string) =>
    side === 'left' || side === 'top' ? start + margin : side === 'right' || side === 'bottom' ? start + size - margin - length : start + (size - length) / 2;
  return { x: place(area.x, area.width, width, horizontal), y: place(area.y, area.height, height, vertical), width, height };
}

export const headerFooterBands: HeaderFooterBand[] = ['header', 'footer'];
export const headerFooterSlots: HeaderFooterSlot[] = ['left', 'center', 'right'];
export const emptyHeaderFooterText = (): HeaderFooterText => ({ header: { left: '', center: '', right: '' }, footer: { left: '', center: '', right: '' } });

// Mirrors header_footer_variant in the native exporter. The first page of the range uses the first-page text,
// even page numbers use the even-page text, and every other page uses the main text.
export function headerFooterVariant(options: HeaderFooterOptions, pageIndex: number, firstIndex: number): HeaderFooterText {
  if (options.firstPage && pageIndex === firstIndex) return options.firstPage;
  if (options.even && (pageIndex + 1) % 2 === 0) return options.even;
  return { header: options.header, footer: options.footer };
}

// The six slots split each band into thirds between the margins; mirrors header_footer_rect in the native exporter.
export function headerFooterRect(area: SceneRect, margin: number, fontSize: number, band: HeaderFooterBand, slot: HeaderFooterSlot): SceneRect {
  const width = Math.max(0, area.width - 2 * margin) / 3, height = fontSize * 1.2;
  return { x: area.x + margin + width * headerFooterSlots.indexOf(slot), y: band === 'header' ? area.y + margin : area.y + area.height - margin - height, width, height };
}

// Header and footer text resolved for the page at pageIndex in the final page order.
export function headerFooterParts(area: SceneRect, object: SceneObject, pageIndex: number, pageCount: number): HeaderFooterPart[] {
  const options = object.headerFooter;
  if (!options) return [];
  let firstIndex = pageIndex;
  try { firstIndex = pagesInRange(object.pageRange ?? 'all', pageCount)[0]; } catch { /* The inspector reports the range. */ }
  const text = headerFooterVariant(options, pageIndex, firstIndex);
  const tokens = { page: pageIndex + 1, pages: pageCount, date: options.date, file: options.fileName };
  return headerFooterBands.flatMap((band) => headerFooterSlots.map((slot) => ({
    band, slot, text: resolvePageTokens(text[band][slot], tokens), rect: headerFooterRect(area, options.margin, object.fontSize, band, slot) })));
}

// Whether any non-empty slot's estimated text box overlaps a text run on the page. Draft text width is estimated.
export function headerFooterOverlaps(parts: HeaderFooterPart[], fontSize: number, runs: Pick<PdfTextRun, 'x' | 'y' | 'width' | 'height'>[]): boolean {
  return parts.some(({ slot, text, rect }) => {
    if (!text.trim()) return false;
    const width = Math.min(rect.width, text.length * fontSize * 0.55);
    const x = slot === 'left' ? rect.x : slot === 'right' ? rect.x + rect.width - width : rect.x + (rect.width - width) / 2;
    return runs.some((run) => run.x < x + width && x < run.x + run.width && run.y < rect.y + rect.height && rect.y < run.y + run.height);
  });
}
