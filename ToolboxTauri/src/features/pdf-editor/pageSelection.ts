import { parsePageRange, selectedPageLabel } from "../../shared/workspaceValidation";
import type { ValidationIssue } from "../../shared/workspaceValidation";
import type { MirrorAxis, PageOrientation, ScenePage } from "./scene";

/** Selected page ids in document order, plus the page a Shift range runs from. */
export type PageSelection = { ids: string[]; anchor: string | null };

export const clearPageSelection = (): PageSelection => ({ ids: [], anchor: null });

const inOrder = (order: string[], ids: Set<string>) => order.filter((id) => ids.has(id));

const span = (order: string[], from: string, to: string) => {
  const start = order.indexOf(from);
  const end = order.indexOf(to);
  return order.slice(Math.min(start, end), Math.max(start, end) + 1);
};

export const togglePage = (order: string[], selection: PageSelection, id: string): PageSelection => {
  const ids = new Set(selection.ids);
  if (ids.has(id)) ids.delete(id);
  else ids.add(id);
  return { ids: inOrder(order, ids), anchor: id };
};

/** Selects the run from the anchor (or the focused page when there is no live anchor) to `id`. */
export const extendPageSelection = (order: string[], selection: PageSelection, focused: string | null, id: string): PageSelection => {
  const anchor = selection.anchor && order.includes(selection.anchor) ? selection.anchor : focused && order.includes(focused) ? focused : id;
  return { ids: span(order, anchor, id), anchor };
};

export const clickPage = (
  order: string[],
  selection: PageSelection,
  id: string,
  { toggle = false, range = false, focused = null }: { toggle?: boolean; range?: boolean; focused?: string | null },
): PageSelection => {
  if (range) return extendPageSelection(order, selection, focused, id);
  if (toggle) return togglePage(order, selection, id);
  return { ids: [id], anchor: id };
};

export const selectAllPages = (order: string[]): PageSelection => ({ ids: [...order], anchor: null });

/** Odd and even refer to current positions, counted from 1. */
export const selectPagesByParity = (order: string[], parity: "odd" | "even"): PageSelection => ({
  ids: order.filter((_, index) => (index % 2 === 0) === (parity === "odd")),
  anchor: null,
});

/** Applies a typed range such as "1-3, 7" to current positions. Blank input selects nothing and returns null. */
export const selectPagesByRange = (order: string[], value: string): PageSelection | ValidationIssue | null => {
  if (!value.split(",").some((part) => part.trim())) return null;
  const positions = parsePageRange(value, order.length);
  if (!Array.isArray(positions)) return positions;
  return { ids: positions.map((index) => order[index]), anchor: null };
};

export const selectionCountLabel = (count: number) => (count === 0 ? "No pages selected" : selectedPageLabel(count));

const quarterTurns = (degrees: number) => ((degrees % 360) + 360) % 360;
const turnWords: Record<number, string> = { 90: "90° right", 180: "180°", 270: "90° left" };
const rotationWords: Record<number, string> = { 90: "rotated 90° right", 180: "rotated 180°", 270: "rotated 90° left" };
const rotationBadges: Record<number, string> = { 90: "↻ 90°", 180: "↻ 180°", 270: "↺ 90°" };
const mirrorWords: Record<MirrorAxis, string> = { horizontal: "mirrored left to right", vertical: "mirrored top to bottom" };
const mirrorBadges: Record<MirrorAxis, string> = { horizontal: "⇋ Mirrored", vertical: "⇵ Mirrored" };

/**
 * Splits an orientation into the turn and flip a person would name. A left-to-right flip turned 180°
 * is a top-to-bottom flip, so that one reads as a single mirror; other mirrored turns keep the
 * left-to-right flip.
 */
const orientationParts = ({ rotation, mirrored }: PageOrientation): { turn: number; mirror: MirrorAxis | null } => {
  const turn = quarterTurns(rotation);
  if (!mirrored) return { turn, mirror: null };
  return turn === 180 ? { turn: 0, mirror: "vertical" } : { turn, mirror: "horizontal" };
};

const orientationWords = (orientation: PageOrientation) => {
  const { turn, mirror } = orientationParts(orientation);
  return [rotationWords[turn], mirror && mirrorWords[mirror]].filter(Boolean).join(", ");
};

/** A page's orientation for its details, such as "Rotated 90° right, mirrored left to right", or "As in the original". */
export const orientationLabel = (orientation: PageOrientation) => {
  const words = orientationWords(orientation);
  return words ? words[0].toUpperCase() + words.slice(1) : "As in the original";
};

/** The short mark a turned or mirrored page shows on its thumbnail, or null when it is as in the original. */
export const orientationBadge = (orientation: PageOrientation) => {
  const { turn, mirror } = orientationParts(orientation);
  return [rotationBadges[turn], mirror && mirrorBadges[mirror]].filter(Boolean).join(" · ") || null;
};

/** Announces a turn of the pages, for example "Pages 4 to 6 rotated 90° left". */
export const rotationAnnouncement = (positions: number[], degrees: number) =>
  `${pagePositionsLabel(positions)} rotated ${turnWords[quarterTurns(degrees)] ?? "0°"}`;

/** Announces a mirror of the pages, for example "Pages 8 and 9 mirrored left to right". */
export const mirrorAnnouncement = (positions: number[], axis: MirrorAxis) => `${pagePositionsLabel(positions)} ${mirrorWords[axis]}`;

/** The notice shown before export while pages are mirrored, or null when none are. */
export const mirroredTextNotice = (count: number) => count === 0 ? null
  : count === 1 ? "1 mirrored page will show its text reversed."
  : `${count} mirrored pages will show their text reversed.`;

const previewWords = { loading: "preview loading", failed: "preview unavailable" } as const;

export const pageOptionName = (position: number, page: Pick<ScenePage, "sourceIndex" | "rotation" | "mirrored">, selected: boolean, preview?: keyof typeof previewWords) => {
  const parts = [`Position ${position}`];
  if (page.sourceIndex !== null) parts.push(`original page ${page.sourceIndex + 1}`);
  const orientation = orientationWords(page);
  if (orientation) parts.push(orientation);
  if (preview) parts.push(previewWords[preview]);
  if (selected) parts.push("selected");
  return parts.join(", ");
};

const listWords = (parts: string[]) => parts.length < 2 ? parts.join("") : `${parts.slice(0, -1).join(", ")} and ${parts[parts.length - 1]}`;

/** Names 1-based positions for an announcement: "Page 5", "Pages 8 and 9", "Pages 4 to 6", "Pages 1 to 3, 7 and 9". */
export const pagePositionsLabel = (positions: number[]) => {
  const sorted = [...new Set(positions)].sort((a, b) => a - b);
  if (sorted.length === 1) return `Page ${sorted[0]}`;
  const parts: string[] = [];
  for (let start = 0; start < sorted.length;) {
    let end = start;
    while (end + 1 < sorted.length && sorted[end + 1] === sorted[end] + 1) end += 1;
    if (end - start >= 2) parts.push(`${sorted[start]} to ${sorted[end]}`);
    else for (let index = start; index <= end; index += 1) parts.push(String(sorted[index]));
    start = end + 1;
  }
  return `Pages ${listWords(parts)}`;
};
