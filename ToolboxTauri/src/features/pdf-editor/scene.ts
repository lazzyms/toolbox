import type { PdfDocument, PdfTextRun } from './contracts';

// All geometry is in points, from the top-left of the unrotated visible source page.
export interface SceneRect { x: number; y: number; width: number; height: number }
export interface ScenePoint { x: number; y: number }
export type SceneTool = 'select' | 'text' | 'highlight' | 'shape' | 'signature' | 'watermark' | 'crop';
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
  // Mirrored left to right before the turn: the page shows rotate(rotation) after a horizontal flip.
  // Top to bottom is the same flip turned 180°, so the pair covers all 8 orientations.
  mirrored?: boolean;
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
}
export const sceneFromDocument = (document: PdfDocument): PdfScene => ({
  pages: document.pages.map((page) => ({ id: `source-${page.index}`, sourceIndex: page.index,
    width: page.width, height: page.height, rotation: 0, mirrored: false, crop: null,
    sourceRotation: page.rotation ?? 0,
    sourceBox: page.pageBox ? { x: page.pageBox[0], y: page.pageBox[1], width: page.pageBox[2] - page.pageBox[0], height: page.pageBox[3] - page.pageBox[1] } : null,
    objects: [] })),
});
export const visibleBounds = (page: ScenePage): SceneRect => page.crop ?? { x: 0, y: 0, width: page.width, height: page.height };

// Page-sequence edits. Each takes a scene and page ids and returns the next scene.
// Returns null when the selection covers every page, because a PDF keeps at least one.
export const removePages = (scene: PdfScene, ids: string[]): PdfScene | null => {
  const pages = scene.pages.filter((page) => !ids.includes(page.id));
  return pages.length ? { pages } : null;
};

// Moves the pages to the target's position, keeping their relative order.
export const movePagesBefore = (scene: PdfScene, ids: string[], targetId: string): PdfScene => {
  if (ids.includes(targetId) || !scene.pages.some((page) => page.id === targetId)) return scene;
  const moving = scene.pages.filter((page) => ids.includes(page.id));
  const pages = scene.pages.filter((page) => !ids.includes(page.id));
  pages.splice(pages.findIndex((page) => page.id === targetId), 0, ...moving);
  return { pages };
};

// Returns the original scene when the order did not change, so callers can skip an undo step.
const reordered = (scene: PdfScene, pages: ScenePage[]): PdfScene =>
  pages.every((page, index) => page === scene.pages[index]) ? scene : { pages };

/** Moves the pages as one block, in their current order, so the block starts at `position` (1-based, clamped). */
export const movePagesTo = (scene: PdfScene, ids: string[], position: number): PdfScene => {
  const moving = scene.pages.filter((page) => ids.includes(page.id));
  if (!moving.length) return scene;
  const pages = scene.pages.filter((page) => !ids.includes(page.id));
  pages.splice(Math.max(0, Math.min(pages.length, Math.round(position) - 1)), 0, ...moving);
  return reordered(scene, pages);
};

/** Moves the pages as one block into a gap of the current order: gap 0 is before the first page, gap n after the last. */
export const movePagesToGap = (scene: PdfScene, ids: string[], gap: number): PdfScene =>
  movePagesTo(scene, ids, scene.pages.slice(0, gap).filter((page) => !ids.includes(page.id)).length + 1);

/** Steps the block one position. The leading edge moves, so scattered pages gather next to the first (earlier) or last (later) one. */
export const movePagesBy = (scene: PdfScene, ids: string[], delta: -1 | 1): PdfScene => {
  const indexes = scene.pages.flatMap((page, index) => ids.includes(page.id) ? [index] : []);
  if (!indexes.length) return scene;
  const start = delta < 0 ? indexes[0] - 1 : indexes[indexes.length - 1] + 2 - indexes.length;
  return movePagesTo(scene, ids, start + 1);
};

/** Reverses the selected pages within the positions they occupy. */
export const reversePages = (scene: PdfScene, ids: string[]): PdfScene => {
  const moving = scene.pages.filter((page) => ids.includes(page.id)).reverse();
  let next = 0;
  return reordered(scene, scene.pages.map((page) => ids.includes(page.id) ? moving[next++] : page));
};

/** Puts a two-sided scan in reading order: fronts were scanned first, then backs in reverse, so the order becomes 1, n, 2, n-1… */
export const interleaveDuplexScan = (scene: PdfScene): PdfScene => {
  const last = scene.pages.length - 1;
  return reordered(scene, scene.pages.map((_, index) => scene.pages[index % 2 ? last - (index - 1) / 2 : index / 2]));
};

export const swapPage = (scene: PdfScene, index: number, delta: number): PdfScene => {
  const to = index + delta;
  if (index < 0 || index >= scene.pages.length || to < 0 || to >= scene.pages.length) return scene;
  const pages = [...scene.pages];
  [pages[index], pages[to]] = [pages[to], pages[index]];
  return { pages };
};

export type MirrorAxis = 'horizontal' | 'vertical';
export interface PageOrientation { rotation: number; mirrored?: boolean }
export type OrientationEdit = { rotate: number } | { mirror: MirrorAxis };

const quarterTurn = (degrees: number) => ((degrees % 360) + 360) % 360;

/**
 * Applies an edit to the orientation as the page is displayed. Turning keeps the flip; a mirror
 * reverses the current turn, because flipping a turned page equals turning it the other way after
 * the flip. Top to bottom is left to right turned 180°, and mirroring twice is the identity.
 */
export const composeOrientation = (orientation: PageOrientation, edit: OrientationEdit): Required<PageOrientation> => {
  const mirrored = !!orientation.mirrored;
  if ('rotate' in edit) return { rotation: quarterTurn(orientation.rotation + edit.rotate), mirrored };
  return { rotation: quarterTurn((edit.mirror === 'vertical' ? 180 : 0) - orientation.rotation), mirrored: !mirrored };
};

const reoriented = (scene: PdfScene, ids: string[], edit: OrientationEdit) =>
  reordered(scene, scene.pages.map((page) => {
    if (!ids.includes(page.id)) return page;
    const next = composeOrientation(page, edit);
    return next.rotation === page.rotation && next.mirrored === !!page.mirrored ? page : { ...page, ...next };
  }));

/**
 * Turns the pages by `degrees` (a multiple of 90, negative is left), on top of any earlier turn, keeping any mirror.
 * `rotation` is the scene's turn only; export adds it to the page's own source rotation. Returns the same scene if nothing turned.
 */
export const rotatePages = (scene: PdfScene, ids: string[], degrees = 90): PdfScene => reoriented(scene, ids, { rotate: degrees });

/** Mirrors the pages left to right (horizontal) or top to bottom (vertical) as they are displayed. */
export const mirrorPages = (scene: PdfScene, ids: string[], axis: MirrorAxis): PdfScene => reoriented(scene, ids, { mirror: axis });

/** Turns and unflips the pages back to their source orientation. Other pages and every other edit stay. Returns the same scene if nothing changed. */
export const resetPageRotation = (scene: PdfScene, ids: string[]): PdfScene =>
  reordered(scene, scene.pages.map((page) => ids.includes(page.id) && (page.rotation || page.mirrored) ? { ...page, rotation: 0, mirrored: false } : page));
