/**
 * Crop geometry for Crop and Resize PDF. A crop rectangle is kept as fractions of the page as the reader
 * sees it (after rotation, top-left origin), so it lands in the same relative spot on every page size;
 * lengths are only for showing and typing, measured against the page in the preview.
 */
export type Unit = "mm" | "in" | "pt";
export type Rect = { left: number; top: number; width: number; height: number };
export type Margins = { top: number; right: number; bottom: number; left: number };
export type PageSize = { width: number; height: number };
export type Handle = "nw" | "n" | "ne" | "e" | "se" | "s" | "sw" | "w";

/** PDF points per unit. */
export const pointsPerUnit: Record<Unit, number> = { mm: 72 / 25.4, in: 72, pt: 1 };

const mm = pointsPerUnit.mm;

/** The board's starting rectangle: 18, 22, 174 × 250 mm on an A4 page. */
export const DEFAULT_RECT: Rect = { left: 18 / 210, top: 22 / 297, width: 174 / 210, height: 250 / 297 };

/** The smallest side a drawn or resized rectangle keeps, as a fraction of the page. */
const MIN_SIDE = 0.01;
/** Slack for fractions that drift while converting to and from lengths. */
const EPSILON = 1e-6;

const clamp = (value: number, min: number, max: number) => Math.min(Math.max(value, min), max);

export const sameRect = (a: Rect, b: Rect) =>
  Math.abs(a.left - b.left) < EPSILON && Math.abs(a.top - b.top) < EPSILON && Math.abs(a.width - b.width) < EPSILON && Math.abs(a.height - b.height) < EPSILON;

export type RectField = keyof Rect;
export type RectIssue = { fields: RectField[]; message: string };

/** Why a rectangle can't crop, as sentences for the inline alert with the fields they concern. Empty when it can. */
export const rectIssues = (rect: Rect): RectIssue[] => {
  const issues: RectIssue[] = [];
  if (!(rect.width > EPSILON)) issues.push({ fields: ["width"], message: "Width must be more than 0." });
  if (!(rect.height > EPSILON)) issues.push({ fields: ["height"], message: "Height must be more than 0." });
  if (rect.left < -EPSILON) issues.push({ fields: ["left"], message: "Left can't be less than 0." });
  if (rect.top < -EPSILON) issues.push({ fields: ["top"], message: "Top can't be less than 0." });
  if (rect.width > EPSILON && rect.left + rect.width > 1 + EPSILON) issues.push({ fields: ["left", "width"], message: "The rectangle goes past the right edge of the page." });
  if (rect.height > EPSILON && rect.top + rect.height > 1 + EPSILON) issues.push({ fields: ["top", "height"], message: "The rectangle goes past the bottom edge of the page." });
  return issues;
};

/** A rectangle spanning two points, clamped to the page. */
export const rectBetween = (a: { x: number; y: number }, b: { x: number; y: number }): Rect => {
  const [x0, x1] = [clamp(Math.min(a.x, b.x), 0, 1), clamp(Math.max(a.x, b.x), 0, 1)];
  const [y0, y1] = [clamp(Math.min(a.y, b.y), 0, 1), clamp(Math.max(a.y, b.y), 0, 1)];
  return { left: x0, top: y0, width: x1 - x0, height: y1 - y0 };
};

/** Moves a rectangle by a fraction of the page, keeping its size and keeping it on the page. */
export const moveRect = (rect: Rect, dx: number, dy: number): Rect => ({
  ...rect,
  left: clamp(rect.left + dx, 0, Math.max(0, 1 - rect.width)),
  top: clamp(rect.top + dy, 0, Math.max(0, 1 - rect.height)),
});

/** Drags one handle to a point; the opposite edges stay put and the rectangle never turns inside out. */
export const resizeRect = (rect: Rect, handle: Handle, point: { x: number; y: number }): Rect => {
  let [left, top, right, bottom] = [rect.left, rect.top, rect.left + rect.width, rect.top + rect.height];
  const x = clamp(point.x, 0, 1);
  const y = clamp(point.y, 0, 1);
  if (handle.includes("w")) left = Math.min(x, right - MIN_SIDE);
  if (handle.includes("e")) right = Math.max(x, left + MIN_SIDE);
  if (handle.includes("n")) top = Math.min(y, bottom - MIN_SIDE);
  if (handle.includes("s")) bottom = Math.max(y, top + MIN_SIDE);
  return { left, top, width: right - left, height: bottom - top };
};

/**
 * Arrow keys move the rectangle 1 mm on the previewed page; with Shift they grow or shrink it 1 mm from
 * its right or bottom edge. Returns null for other keys.
 */
export const nudgeRect = (rect: Rect, key: string, shift: boolean, page: PageSize): Rect | null => {
  const stepX = mm / page.width;
  const stepY = mm / page.height;
  const delta = { ArrowLeft: [-stepX, 0], ArrowRight: [stepX, 0], ArrowUp: [0, -stepY], ArrowDown: [0, stepY] }[key];
  if (!delta) return null;
  if (!shift) return moveRect(rect, delta[0], delta[1]);
  return {
    ...rect,
    width: clamp(rect.width + delta[0], Math.min(stepX, 1 - rect.left), Math.max(stepX, 1 - rect.left)),
    height: clamp(rect.height + delta[1], Math.min(stepY, 1 - rect.top), Math.max(stepY, 1 - rect.top)),
  };
};

/** The kept area on a page, in points, or null when nothing is left. */
export const keptSize = (page: PageSize, crop: { rect: Rect } | { margins: Margins }): PageSize | null => {
  const size = "rect" in crop
    ? { width: crop.rect.width * page.width, height: crop.rect.height * page.height }
    : { width: page.width - crop.margins.left - crop.margins.right, height: page.height - crop.margins.top - crop.margins.bottom };
  return size.width > EPSILON && size.height > EPSILON ? size : null;
};

/** The kept area as fractions of a page, for drawing it. Null when nothing is left. */
export const keptRect = (page: PageSize, crop: { rect: Rect } | { margins: Margins }): Rect | null => {
  if ("rect" in crop) return crop.rect;
  if (!keptSize(page, crop)) return null;
  const { top, right, bottom, left } = crop.margins;
  return { left: left / page.width, top: top / page.height, width: (page.width - left - right) / page.width, height: (page.height - top - bottom) / page.height };
};

const papers: ReadonlyArray<{ name: string; width: number; height: number }> = [
  { name: "A3", width: 841.89, height: 1190.55 },
  { name: "A4", width: 595.28, height: 841.89 },
  { name: "A5", width: 419.53, height: 595.28 },
  { name: "Letter", width: 612, height: 792 },
  { name: "Legal", width: 612, height: 1008 },
  { name: "Tabloid", width: 792, height: 1224 },
];

/** "A4", "Letter, landscape", or null for a size without a common name. */
export const paperName = (page: PageSize): string | null => {
  const near = (a: number, b: number) => Math.abs(a - b) < 1.5;
  for (const paper of papers) {
    if (near(page.width, paper.width) && near(page.height, paper.height)) return paper.name;
    if (near(page.width, paper.height) && near(page.height, paper.width)) return `${paper.name}, landscape`;
  }
  return null;
};

const trim = (value: number, decimals: number) => String(Number(value.toFixed(decimals)));

/** A rounded length for captions: whole millimetres and points, inches to two places. */
export const showLength = (points: number, unit: Unit) => trim(points / pointsPerUnit[unit], unit === "in" ? 2 : 0);

/** A length for a text field: a tenth of a millimetre or point, a hundredth of an inch. */
export const fieldLength = (points: number, unit: Unit) => trim(points / pointsPerUnit[unit], unit === "in" ? 2 : 1);

export const showSize = (size: PageSize, unit: Unit, separator = "×") => `${showLength(size.width, unit)} ${separator} ${showLength(size.height, unit)} ${unit}`;

/** "A4, 210 × 297 mm", or just the size when it has no common name. */
export const describePage = (page: PageSize, unit: Unit) => {
  const name = paperName(page);
  return name ? `${name}, ${showSize(page, unit)}` : showSize(page, unit);
};

/** A key that treats sizes within a point as the same and keeps orientation apart. */
export const sizeKey = (page: PageSize) => `${Math.round(page.width)}x${Math.round(page.height)}`;
