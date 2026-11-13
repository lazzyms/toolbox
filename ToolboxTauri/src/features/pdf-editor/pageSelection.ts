import { parsePageRange, selectedPageLabel } from "../../shared/workspaceValidation";
import type { ValidationIssue } from "../../shared/workspaceValidation";
import type { ScenePage } from "./scene";

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

const orientationWords: Record<number, string> = { 90: "rotated 90° right", 180: "rotated 180°", 270: "rotated 90° left" };

const previewWords = { loading: "preview loading", failed: "preview unavailable" } as const;

export const pageOptionName = (position: number, page: Pick<ScenePage, "sourceIndex" | "rotation">, selected: boolean, preview?: keyof typeof previewWords) => {
  const parts = [`Position ${position}`];
  if (page.sourceIndex !== null) parts.push(`original page ${page.sourceIndex + 1}`);
  const orientation = orientationWords[((page.rotation % 360) + 360) % 360];
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
