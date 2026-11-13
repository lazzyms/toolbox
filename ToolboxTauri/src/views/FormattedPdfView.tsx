import { useContext, useEffect, useRef, useState, type ClipboardEvent, type KeyboardEvent, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { ArrowDown, ArrowUp, CircleX, Image as ImageIcon, ImageOff, List, ListOrdered, Plus, SeparatorHorizontal, Table, Trash2 } from "lucide-react";
import type { JobOutcome, ToolDefinition } from "../contracts";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import { Separator } from "@/components/ui/separator";
import { Toggle } from "@/components/ui/toggle";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { ResultList } from "../components/ResultList";
import { FormattedPdfPreview, useFormattedPreview } from "./FormattedPdfPreview";
import { RunStateContext } from "../components/ToolScaffold";
import { ShortcutHint } from "../components/ShortcutHint";
import { chooseCreatedDocumentSavePath } from "../components/saveStep";
import { failedOutcome, parseToolResult } from "../jobOutcome";
import { TOOL_DROP_EVENT, toolDrop, type DropPoint } from "../hooks/useShellBridge";
import { formatKeyAria, formatKeyLabel, isWindows, matchFormatKey, shortcutTarget, type FormatKeyId } from "../shortcuts";
import {
  RichField, insertIntoRuns, normalizeRuns, plainRuns, runsText, selectionOffsets, setSelectionOffsets, sliceRuns, splitRunLines, type Run,
} from "./FormattedPdfRichField";

type TextKind = "heading1" | "heading2" | "heading3" | "paragraph";
type ListKind = "bulletedList" | "numberedList";
export type BlockKind = TextKind | ListKind | "table" | "image" | "pageBreak";

/** A list item or a table cell: one field of text. */
type Item = { id: number; runs: Run[] };
type Block =
  | { id: number; kind: TextKind; runs: Run[] }
  | { id: number; kind: ListKind; items: Item[] }
  | { id: number; kind: "table"; headerRow: boolean; rows: Item[][] }
  /** A picture on the user's disk, read when the PDF is saved. */
  | { id: number; kind: "image"; path: string }
  | { id: number; kind: "pageBreak" };
type TableBlock = Extract<Block, { kind: "table" }>;

const kindLabels: Record<BlockKind, string> = {
  heading1: "Heading 1", heading2: "Heading 2", heading3: "Heading 3", paragraph: "Paragraph",
  bulletedList: "Bulleted list", numberedList: "Numbered list", table: "Table", image: "Image", pageBreak: "Page break",
};
const gutterLabels: Record<BlockKind, string> = {
  heading1: "H1", heading2: "H2", heading3: "H3", paragraph: "Text", bulletedList: "List", numberedList: "List", table: "Table", image: "Image", pageBreak: "Break",
};
const typeChoices: readonly (Exclude<BlockKind, "table" | "image" | "pageBreak"> & FormatKeyId)[] = ["paragraph", "heading1", "heading2", "heading3", "bulletedList", "numberedList"];

type PageSize = "a4" | "letter" | "legal";
type Orientation = "portrait" | "landscape";
type Margins = "narrow" | "normal" | "wide";
/** How every page looks; sent with the document as its page settings. */
export type PageSettings = { pageSize: PageSize; orientation: Orientation; margins: Margins; header: string; footer: string; pageNumbers: boolean };
const defaultPage: PageSettings = { pageSize: "a4", orientation: "portrait", margins: "normal", header: "", footer: "", pageNumbers: false };
/** Each size's name, its choice in the menu, and its portrait width and height in points. */
const pageSizes: Record<PageSize, { name: string; choice: string; points: readonly [number, number] }> = {
  a4: { name: "A4", choice: "A4, 210 × 297 mm", points: [595, 842] },
  letter: { name: "Letter", choice: "Letter, 8.5 × 11 in", points: [612, 792] },
  legal: { name: "Legal", choice: "Legal, 8.5 × 14 in", points: [612, 1008] },
};
const marginLabels: Record<Margins, { name: string; hint: string }> = {
  narrow: { name: "Narrow", hint: "Narrow is 13 mm on every side." },
  normal: { name: "Normal", hint: "Normal is 25 mm on every side." },
  wide: { name: "Wide", hint: "Wide is 38 mm on every side." },
};
const pageSummary = (page: PageSettings) => `${pageSizes[page.pageSize].name}, ${page.orientation}, ${page.margins} margins`;
const bandSummary = (page: PageSettings) => [
  page.header.trim() && "Header", page.footer.trim() && "Footer", page.pageNumbers && "Page numbers",
].filter(Boolean).join(", ").toLowerCase().replace(/^./, (first) => first.toUpperCase()) || "None";
/** The font this version always uses; the font picker comes later. */
const FONT = { family: "Liberation Serif", size: 11 } as const;

let lastId = 0;
const nextId = () => ++lastId;
const textBlock = (kind: TextKind = "paragraph", runs: Run[] = []): Block => ({ id: nextId(), kind, runs: kind === "paragraph" ? runs : plainRuns(runsText(runs)) });
const item = (runs: Run[] = []): Item => ({ id: nextId(), runs });
const pageBreak = (): Block => ({ id: nextId(), kind: "pageBreak" });
/** The most a table may hold, as create_formatted_pdf checks it. */
const MAX_TABLE_COLUMNS = 20;
const MAX_TABLE_ROWS = 1000;
/** A new table has three columns, a header row and two rows under it. */
const IMAGE_EXTENSIONS = ["png", "jpg", "jpeg", "webp", "heic", "heif", "tif", "tiff"];
const fileName = (path: string) => path.split(/[\\/]/).pop() || path;
/** Whether a file is a picture create_formatted_pdf can place, judged by its extension as the native side judges it. */
const isImagePath = (path: string) => {
  const extension = fileName(path).split(".").slice(1).pop();
  return extension !== undefined && IMAGE_EXTENSIONS.includes(extension.toLowerCase());
};
const imageBlock = (path: string): Block => ({ id: nextId(), kind: "image", path });
const table = (): TableBlock => ({ id: nextId(), kind: "table", headerRow: true, rows: Array.from({ length: 3 }, () => Array.from({ length: 3 }, () => item())) });

const isList = (block: Block): block is Extract<Block, { kind: ListKind }> => block.kind === "bulletedList" || block.kind === "numberedList";
const isTable = (block: Block): block is TableBlock => block.kind === "table";
/** A block with no text of its own: a page break or an image. */
const isFixed = (block: Block): block is Extract<Block, { kind: "pageBreak" | "image" }> => block.kind === "pageBreak" || block.kind === "image";
const isText = (block: Block): block is Extract<Block, { kind: TextKind }> => !isFixed(block) && !isList(block) && !isTable(block);
const blank = (runs: readonly Run[]) => runsText(runs).trim() === "";
/** A block's fields in order: a list's items, or a table's cells row by row. */
const fieldsOf = (block: Extract<Block, { kind: ListKind | "table" }>) => isList(block) ? block.items : block.rows.flat();
const blockIsBlank = (block: Block) => block.kind === "pageBreak" || (block.kind !== "image" && (isText(block) ? blank(block.runs) : fieldsOf(block).every((entry) => blank(entry.runs))));
const firstKey = (block: Block) => String(isList(block) || isTable(block) ? fieldsOf(block)[0].id : block.id);

/** Where a field sits: the block, and the list item (or table cell, counted row by row) for list and table blocks. */
type Place = { index: number; block: Block; itemIndex: number | null };
const locate = (blocks: readonly Block[], key: string | null): Place | null => {
  if (key === null) return null;
  for (const [index, block] of blocks.entries()) {
    if (isList(block) || isTable(block)) {
      const itemIndex = fieldsOf(block).findIndex((entry) => String(entry.id) === key);
      if (itemIndex !== -1) return { index, block, itemIndex };
    } else if (String(block.id) === key) return { index, block, itemIndex: null };
  }
  return null;
};
const placeKey = ({ block, itemIndex }: Place) => String(isList(block) || isTable(block) ? fieldsOf(block)[itemIndex ?? 0].id : block.id);
const placeRuns = (place: Place): Run[] =>
  isFixed(place.block) ? [] : isText(place.block) ? place.block.runs : fieldsOf(place.block)[place.itemIndex ?? 0].runs;

/** The first number each numbered list shows: numbering runs on across page breaks, as the PDF numbers it. */
const listStarts = (blocks: readonly Block[]) => {
  let next: number | null = null;
  return blocks.map((block) => {
    if (blockIsBlank(block)) return null;
    if (block.kind !== "numberedList") { next = null; return null; }
    const start: number = next ?? 1;
    next = start + block.items.filter((entry) => !blank(entry.runs)).length;
    return start;
  });
};

/** The document blocks create_formatted_pdf takes (`DocumentBlock` in kit/pdf/formatted.rs). */
type RequestBlock =
  | { kind: Exclude<TextKind, "paragraph">; text: string }
  | { kind: "paragraph"; runs: Run[] }
  | { kind: ListKind; items: { runs: Run[] }[] }
  | { kind: "table"; headerRow: boolean; rows: string[][] }
  | { kind: "image"; path: string }
  | { kind: "pageBreak" };

/** A table's cell text, without its blank rows and columns; the header row stays only while its row is kept. */
const requestTable = (block: TableBlock): RequestBlock[] => {
  const text = block.rows.map((row) => row.map((cell) => runsText(cell.runs).trim()));
  const columns = text[0].map((_, column) => column).filter((column) => text.some((row) => row[column] !== ""));
  const rows = text.map((row, at) => ({ at, cells: columns.map((column) => row[column]) })).filter(({ cells }) => cells.some((cell) => cell !== ""));
  if (rows.length === 0) return [];
  return [{ kind: "table", headerRow: block.headerRow && rows[0].at === 0, rows: rows.map(({ cells }) => cells) }];
};

/** What create_formatted_pdf receives: blank blocks and items are left out, headings carry their text only. */
const requestBlocks = (blocks: readonly Block[]) => blocks.flatMap((block): RequestBlock[] => {
  if (block.kind === "pageBreak") return [{ kind: "pageBreak" }];
  if (block.kind === "image") return [{ kind: "image", path: block.path }];
  if (isTable(block)) return requestTable(block);
  if (isList(block)) {
    const items = block.items.filter((entry) => !blank(entry.runs)).map((entry) => ({ runs: normalizeRuns(entry.runs) }));
    return items.length > 0 ? [{ kind: block.kind, items }] : [];
  }
  if (blank(block.runs)) return [];
  return block.kind === "paragraph" ? [{ kind: block.kind, runs: normalizeRuns(block.runs) }] : [{ kind: block.kind, text: runsText(block.runs) }];
});

/** Pasted text with blank lines between paragraphs becomes one block (or list item) for each. */
const pastedParts = (text: string, lines: boolean) =>
  text.replace(/\r\n?/g, "\n").split(lines ? /\n+/ : /\n[ \t]*\n+/).map((part) => part.trim()).filter(Boolean);

/** Replaces the selection with `parts`: the first joins the text before the selection, the last the text after it. */
const spliceParts = (runs: readonly Run[], start: number, end: number, parts: readonly string[]): Run[][] => {
  const before = sliceRuns(runs, 0, start);
  const after = sliceRuns(runs, end);
  if (parts.length === 1) return [insertIntoRuns([...before, ...after], start, parts[0])];
  return [insertIntoRuns(before, start, parts[0]), ...parts.slice(1, -1).map(plainRuns), normalizeRuns([...plainRuns(parts[parts.length - 1]), ...after])];
};

type Selection = { key: string; start: number; end: number };

/** The image a failed save names as missing, by its path in the message ("… It was at {path}. …"). */
const missingImage = (outcomes: readonly JobOutcome[], blocks: readonly Block[]) => {
  const failure = outcomes.find((outcome) => outcome.state === "failed")?.failure;
  if (failure?.kind !== "invalidInput") return null;
  const paths = blocks.flatMap((block) => block.kind === "image" && failure.message.includes(`It was at ${block.path}.`) ? [block.path] : []);
  return paths.sort((a, b) => b.length - a.length)[0] ?? null;
};

export const FormattedPdfView = ({ utility }: { utility: ToolDefinition }) => {
  const [blocks, setBlocks] = useState<Block[]>(() => [textBlock()]);
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [emphasis, setEmphasis] = useState({ bold: false, italic: false });
  const [title, setTitle] = useState("");
  const [author, setAuthor] = useState("");
  const [page, setPage] = useState<PageSettings>(defaultPage);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState<JobOutcome[] | null>(null);
  /** The path of an image the last save couldn't find; it stays flagged while a block still points there. */
  const [missing, setMissing] = useState<string | null>(null);
  const list = useRef<HTMLOListElement>(null);
  const focusNext = useRef<Selection | null>(null);
  const lastSelection = useRef<Selection | null>(null);
  const reportRunning = useContext(RunStateContext);

  const fieldFor = (key: string) => list.current?.querySelector<HTMLElement>(`[data-field-key="${key}"]`) ?? null;

  const focusField = (next: Selection) => {
    const field = fieldFor(next.key);
    if (!field) return;
    field.focus();
    if (field.isContentEditable) setSelectionOffsets(field, next.start, next.end);
    lastSelection.current = next;
    setSelectedKey(next.key);
  };

  useEffect(() => {
    const next = focusNext.current;
    if (!next) return;
    focusNext.current = null;
    focusField(next);
  }, [blocks]);

  // The toolbar acts on the field the caret was last in, and shows whether bold and italic are on there.
  useEffect(() => {
    const track = () => {
      const anchor = window.getSelection()?.anchorNode;
      const field = (anchor instanceof Element ? anchor : anchor?.parentElement)?.closest<HTMLElement>("[data-field-key]");
      if (!field || !list.current?.contains(field)) return;
      const offsets = selectionOffsets(field);
      if (offsets) lastSelection.current = { key: field.dataset.fieldKey!, ...offsets };
      const plain = field.dataset.plain === "true";
      setEmphasis({ bold: !plain && document.queryCommandState("bold"), italic: !plain && document.queryCommandState("italic") });
    };
    document.addEventListener("selectionchange", track);
    return () => document.removeEventListener("selectionchange", track);
  }, []);

  const currentPlace = locate(blocks, selectedKey) ?? { index: 0, block: blocks[0], itemIndex: isList(blocks[0]) ? 0 : null };
  const current = currentPlace.block;
  const empty = blocks.every(blockIsBlank);
  const setPageSetting = <K extends keyof PageSettings>(key: K, value: PageSettings[K]) => setPage((current) => ({ ...current, [key]: value }));
  const starts = listStarts(blocks);

  const replace = (index: number, replacement: Block[], focus: Selection) => {
    focusNext.current = focus;
    setBlocks((all) => [...all.slice(0, index), ...replacement, ...all.slice(index + 1)]);
  };

  const setRuns = (key: string, runs: Run[]) => setBlocks((all) => all.map((block) => {
    if (String(block.id) === key && isText(block)) return { ...block, runs };
    if (isTable(block)) return block.rows.some((row) => row.some((cell) => String(cell.id) === key))
      ? { ...block, rows: block.rows.map((row) => row.map((cell) => String(cell.id) === key ? { ...cell, runs } : cell)) }
      : block;
    if (!isList(block) || !block.items.some((entry) => String(entry.id) === key)) return block;
    return { ...block, items: block.items.map((entry) => String(entry.id) === key ? { ...entry, runs } : entry) };
  }));

  /** Replaces one list item with `items`, keeping the rest of the list. */
  const withItems = (block: Extract<Block, { kind: ListKind }>, at: number, items: Item[]): Block => ({ ...block, items: [...block.items.slice(0, at), ...items, ...block.items.slice(at + 1)] });

  /** Enter: ends the block or list item at the caret. Enter on an empty list item ends the list. */
  const split = (place: Place, start: number, end: number) => {
    const { index, block, itemIndex } = place;
    const runs = placeRuns(place);
    if (isList(block) && itemIndex !== null) {
      if (blank(runs)) {
        const before = block.items.slice(0, itemIndex);
        const after = block.items.slice(itemIndex + 1);
        const paragraph = textBlock();
        replace(index, [
          ...(before.length > 0 ? [{ ...block, items: before }] : []),
          paragraph,
          ...(after.length > 0 ? [{ id: nextId(), kind: block.kind, items: after }] : []),
        ], { key: String(paragraph.id), start: 0, end: 0 });
        return;
      }
      const added = item(sliceRuns(runs, end));
      replace(index, [withItems(block, itemIndex, [{ ...block.items[itemIndex], runs: sliceRuns(runs, 0, start) }, added])], { key: String(added.id), start: 0, end: 0 });
      return;
    }
    if (!isText(block)) return;
    const added = textBlock("paragraph", sliceRuns(runs, end));
    replace(index, [{ ...block, runs: sliceRuns(runs, 0, start) }, added], { key: String(added.id), start: 0, end: 0 });
  };

  /** Shift+Enter, and pasted text, replace the selection inside the field; pasted paragraphs become blocks or items. */
  const insertParts = (place: Place, key: string, start: number, end: number, parts: readonly string[]) => {
    const { index, block, itemIndex } = place;
    const pieces = spliceParts(placeRuns(place), start, end, parts);
    const caret = parts.length === 1 ? start + parts[0].length : parts[parts.length - 1].length;
    if (isList(block) && itemIndex !== null) {
      const items = pieces.map((runs, at) => at === 0 ? { ...block.items[itemIndex], runs } : item(runs));
      replace(index, [withItems(block, itemIndex, items)], { key: String(items[items.length - 1].id), start: caret, end: caret });
      return;
    }
    if (!isText(block)) return;
    const added = pieces.slice(1).map((runs) => textBlock("paragraph", runs));
    const last = added[added.length - 1];
    replace(index, [{ ...block, runs: block.kind === "paragraph" ? pieces[0] : plainRuns(runsText(pieces[0])) }, ...added], { key: last ? String(last.id) : key, start: caret, end: caret });
  };

  const paste = (place: Place, key: string, event: ClipboardEvent<HTMLDivElement>) => {
    event.preventDefault();
    const text = event.clipboardData.getData("text/plain");
    const offsets = selectionOffsets(event.currentTarget);
    if (!offsets || text === "") return;
    // A table cell holds one line, so pasted lines join with spaces.
    if (isTable(place.block)) { document.execCommand("insertText", false, text.replace(/\s*[\r\n]+\s*/g, " ")); return; }
    // Text on one line is typed in by the browser, so it keeps the style at the caret and the browser's undo.
    if (!/[\r\n]/.test(text)) { document.execCommand("insertText", false, text); return; }
    const parts = pastedParts(text, place.itemIndex !== null);
    if (parts.length > 0) insertParts(place, key, offsets.start, offsets.end, parts);
  };

  /** Applies a block type: headings and paragraphs keep the caret, lists take one item per line and give it back. */
  const changeKind = (place: Place, start: number, end: number, kind: Exclude<BlockKind, "table" | "image" | "pageBreak">) => {
    const { index, block, itemIndex } = place;
    if (isFixed(block) || isTable(block) || block.kind === kind) return;
    if (isList(block)) {
      if (kind === "bulletedList" || kind === "numberedList") {
        replace(index, [{ ...block, kind }], { key: placeKey(place), start, end });
        return;
      }
      const made = block.items.map((entry) => textBlock(kind, entry.runs));
      replace(index, made, { key: String(made[itemIndex ?? 0].id), start, end });
      return;
    }
    if (kind === "bulletedList" || kind === "numberedList") {
      const lines = splitRunLines(block.runs);
      const items = lines.map((runs) => item(runs));
      let line = 0;
      let lineStart = 0;
      while (line < lines.length - 1 && start > lineStart + runsText(lines[line]).length) { lineStart += runsText(lines[line]).length + 1; line += 1; }
      const caret = start - lineStart;
      replace(index, [{ id: block.id, kind, items }], { key: String(items[line].id), start: caret, end: caret });
      return;
    }
    replace(index, [{ ...block, kind, runs: kind === "paragraph" ? block.runs : plainRuns(runsText(block.runs)) }], { key: String(block.id), start, end });
  };

  /** Starts a new page at the caret: a block (or list) is split there, and a numbered list keeps counting after it. */
  const insertPageBreak = (place: Place, start: number, end: number) => {
    const { index, block, itemIndex } = place;
    const added = pageBreak();
    if (isFixed(block) || isTable(block)) { replace(index, [block, added], { key: String(added.id), start: 0, end: 0 }); return; }
    const runs = placeRuns(place);
    const before = sliceRuns(runs, 0, start);
    const after = sliceRuns(runs, end);
    if (isList(block) && itemIndex !== null) {
      const current = block.items[itemIndex];
      const head = [...block.items.slice(0, itemIndex), ...(blank(before) ? [] : [{ ...current, runs: before }])];
      const first = blank(before) ? { ...current, runs: after } : item(after);
      const tail = [first, ...block.items.slice(itemIndex + 1)];
      const focus = { key: String(first.id), start: 0, end: 0 };
      if (head.length === 0) replace(index, [added, { ...block, items: tail }], focus);
      else replace(index, [{ ...block, items: head }, added, { id: nextId(), kind: block.kind, items: tail }], focus);
      return;
    }
    if (!isText(block)) return;
    if (blank(before)) { replace(index, [added, { ...block, runs: after }], { key: String(block.id), start: 0, end: 0 }); return; }
    const rest = textBlock("paragraph", after);
    replace(index, [{ ...block, runs: before }, added, rest], { key: String(rest.id), start: 0, end: 0 });
  };

  /** Adds a table after the block the caret was last in, or in place of an empty paragraph, with the caret in its first cell. */
  const insertTable = () => {
    const remembered = lastSelection.current;
    const { index, block } = (remembered && locate(blocks, remembered.key)) || currentPlace;
    const added = table();
    const focus = { key: firstKey(added), start: 0, end: 0 };
    if (isText(block) && blockIsBlank(block)) replace(index, [added], focus);
    else replace(index, [block, added], focus);
  };

  /**
   * Adds images, in order, next to a block: after it, or before it when `before` is set. An empty paragraph is
   * replaced. Without a block, they go after the block the caret was last in. Focus moves to the last image.
   */
  const insertImages = (paths: readonly string[], target: { index: number; before: boolean } | null = null) => {
    if (paths.length === 0) return;
    const remembered = lastSelection.current;
    const { index, before } = target ?? { index: ((remembered && locate(blocks, remembered.key)) || currentPlace).index, before: false };
    const block = blocks[index];
    const added = paths.map(imageBlock);
    focusNext.current = { key: String(added[added.length - 1].id), start: 0, end: 0 };
    if (isText(block) && blockIsBlank(block)) { setBlocks([...blocks.slice(0, index), ...added, ...blocks.slice(index + 1)]); return; }
    const at = before ? index : index + 1;
    setBlocks([...blocks.slice(0, at), ...added, ...blocks.slice(at)]);
  };

  const pickImage = async () => {
    const picked = await open({ multiple: false, directory: false, filters: [{ name: "Image", extensions: IMAGE_EXTENSIONS }] });
    return typeof picked === "string" ? picked : null;
  };

  const addPickedImage = async () => {
    const path = await pickImage();
    if (path) insertImages([path]);
  };

  /** Points an image block at another file, as Choose image… on a missing image does. */
  const replaceImage = async (index: number, block: Extract<Block, { kind: "image" }>) => {
    const path = await pickImage();
    if (path) replace(index, [{ ...block, path }], { key: String(block.id), start: 0, end: 0 });
  };

  /** Images dropped from Finder or File Explorer go where they were dropped: before the block under the upper half of it, after it otherwise. */
  const dropImages = (paths: readonly string[], point: DropPoint | null) => {
    const images = paths.filter(isImagePath);
    if (saving || images.length === 0) return;
    const under = point ? document.elementFromPoint(point.x, point.y)?.closest<HTMLElement>(".formatted-pdf-block") : null;
    const index = under && list.current?.contains(under) ? Array.from(list.current.children).indexOf(under) : -1;
    if (index === -1) { insertImages(images); return; }
    const box = under!.getBoundingClientRect();
    insertImages(images, { index, before: point!.y < box.top + box.height / 2 });
  };
  const dropRef = useRef(dropImages);
  dropRef.current = dropImages;
  useEffect(() => {
    const listener = (event: Event) => {
      const drop = toolDrop(event);
      if (drop) dropRef.current(drop.paths, drop.point);
    };
    window.addEventListener(TOOL_DROP_EVENT, listener);
    return () => window.removeEventListener(TOOL_DROP_EVENT, listener);
  }, []);

  /** Adds a row under a table's last row, or a column after its last column, with the caret in the first new cell. */
  const growTable = (index: number, block: TableBlock, add: "row" | "column") => {
    const rows = add === "row" ? [...block.rows, block.rows[0].map(() => item())] : block.rows.map((row) => [...row, item()]);
    const first = add === "row" ? rows[rows.length - 1][0] : rows[0][rows[0].length - 1];
    replace(index, [{ ...block, rows }], { key: String(first.id), start: 0, end: 0 });
  };

  const setHeaderRow = (index: number, headerRow: boolean) =>
    setBlocks((all) => all.map((block, at) => at === index && isTable(block) ? { ...block, headerRow } : block));

  /** Moves a block; moved with the keyboard, the caret stays where it was. */
  const move = (index: number, by: -1 | 1, focus: Selection | null = null) => {
    const target = index + by;
    if (target < 0 || target >= blocks.length) return;
    focusNext.current = focus;
    setBlocks((all) => {
      const next = [...all];
      [next[index], next[target]] = [next[target], next[index]];
      return next;
    });
  };

  const remove = (index: number) => {
    const next = blocks.length === 1 ? [textBlock()] : blocks.filter((_, at) => at !== index);
    const focus = next[Math.min(index, next.length - 1)];
    const key = firstKey(focus);
    const field = locate(next, key);
    const length = field ? runsText(placeRuns(field)).length : 0;
    focusNext.current = { key, start: length, end: length };
    setBlocks(next);
  };

  /** Bold and italic go through the browser's own editing, so they apply to the selection or to what is typed next. */
  const emphasize = (place: Place, style: "bold" | "italic") => {
    if (place.block.kind !== "paragraph" && !isList(place.block)) return;
    document.execCommand(style);
    setEmphasis({ bold: document.queryCommandState("bold"), italic: document.queryCommandState("italic") });
  };

  const act = (id: FormatKeyId, place: Place, start: number, end: number) => {
    switch (id) {
      case "bold":
      case "italic": emphasize(place, id); break;
      case "heading1":
      case "heading2":
      case "heading3":
      case "paragraph": changeKind(place, start, end, id); break;
      case "bulletedList":
      case "numberedList": changeKind(place, start, end, place.block.kind === id ? "paragraph" : id); break;
      case "pageBreak": insertPageBreak(place, start, end); break;
      case "moveUp":
      case "moveDown": move(place.index, id === "moveUp" ? -1 : 1, { key: placeKey(place), start, end }); break;
    }
  };

  /** Toolbar controls act where the caret last was, putting focus and the selection back first. */
  const fromToolbar = (id: FormatKeyId) => {
    const remembered = lastSelection.current;
    const last = remembered && locate(blocks, remembered.key) ? remembered : { key: placeKey(currentPlace), start: 0, end: 0 };
    const place = locate(blocks, last.key) ?? currentPlace;
    const field = fieldFor(last.key);
    if (field && document.activeElement !== field) {
      field.focus();
      if (field.isContentEditable) setSelectionOffsets(field, last.start, last.end);
    }
    act(id, place, last.start, last.end);
  };

  const fieldKeyDown = (place: Place, key: string, event: KeyboardEvent<HTMLElement>) => {
    const native = event.nativeEvent;
    if (native.isComposing || native.keyCode === 229) return;
    const field = event.currentTarget;
    const offsets = field.isContentEditable ? selectionOffsets(field) ?? { start: 0, end: 0 } : { start: 0, end: 0 };
    const id = matchFormatKey(native);
    if (id) {
      event.preventDefault();
      act(id, place, offsets.start, offsets.end);
      return;
    }
    if (isFixed(place.block)) {
      if (event.key === "Delete" || event.key === "Backspace") { event.preventDefault(); remove(place.index); }
      return;
    }
    if (event.key === "Enter" && !event.metaKey && !event.ctrlKey && !event.altKey) {
      event.preventDefault();
      if (event.shiftKey) insertParts(place, key, offsets.start, offsets.end, ["\n"]);
      else split(place, offsets.start, offsets.end);
      return;
    }
    // Underline is the browser's own key in editable text, and a PDF run has no underline.
    if ((event.metaKey || event.ctrlKey) && event.code === "KeyU") event.preventDefault();
  };

  // The preview gets the document a save would send, without the output path.
  const typed = { title: title.trim(), author: author.trim(), blocks: requestBlocks(blocks), page, font: FONT };
  const { preview, updating, error: previewError } = useFormattedPreview(typed.blocks.length > 0 ? typed : null);
  const [portraitWidth, portraitHeight] = pageSizes[page.pageSize].points;
  const previewSize = page.orientation === "portrait" ? [portraitWidth, portraitHeight] as const : [portraitHeight, portraitWidth] as const;
  const estimate = preview && !updating && preview.pageCount > 0
    ? `${preview.pageCount === 1 ? "1 page" : `${preview.pageCount} pages`}, ${pageSizes[page.pageSize].name}${page.orientation === "landscape" ? " landscape" : ""}`
    : null;

  const save = async () => {
    if (empty || saving) return;
    const name = `${title.trim() || "Untitled"}.pdf`;
    const outputPath = await chooseCreatedDocumentSavePath({ title: "Save the PDF", extension: "pdf" }, name, `${pageSizes[page.pageSize].name}${page.orientation === "landscape" ? " landscape" : ""}, Liberation Serif 11 pt`);
    if (!outputPath) return;
    setSaving(true);
    setMissing(null);
    reportRunning?.(true);
    try {
      const request = { document: typed, outputPath };
      const outcomes = parseToolResult(await invoke("create_formatted_pdf", { request }));
      // A missing image keeps the draft open on the block to fix, as the board shows, with nothing to show in results.
      const lost = missingImage(outcomes, blocks);
      setMissing(lost);
      setSaved(lost ? null : outcomes);
    } catch (error) {
      setSaved([failedOutcome(outputPath, "processing", error instanceof Error ? error.message : String(error))]);
    } finally {
      setSaving(false);
      reportRunning?.(false);
    }
  };

  const startOver = () => {
    const first = textBlock();
    setBlocks([first]);
    setSelectedKey(null);
    setTitle("");
    setAuthor("");
    setPage(defaultPage);
    setSaved(null);
    setMissing(null);
    lastSelection.current = null;
    focusNext.current = { key: String(first.id), start: 0, end: 0 };
  };

  const richOff = saving || (current.kind !== "paragraph" && !isList(current));
  const fixedKind = isFixed(current) || isTable(current);
  const missingBlock = missing === null ? null : blocks.find((block) => block.kind === "image" && block.path === missing) ?? null;
  const missingName = missing === null ? "" : fileName(missing);
  const keepFocus = (event: { preventDefault: () => void }) => event.preventDefault();
  const tool = (id: FormatKeyId, label: string, control: ReactNode) => (
    <Tooltip>
      <TooltipTrigger asChild>{control}</TooltipTrigger>
      <TooltipContent>{label} <kbd className="opacity-70">{formatKeyLabel(id)}</kbd></TooltipContent>
    </Tooltip>
  );

  const field = (block: Block, index: number, key: string, label: string, runs: Run[], plain: boolean, itemIndex: number | null) => {
    const place = { index, block, itemIndex };
    const cell = isTable(block);
    return (
      <RichField
        fieldKey={key}
        label={label}
        runs={runs}
        plain={plain}
        singleLine={cell}
        disabled={saving}
        describedBy="formatted-pdf-typing-hint"
        className={block.kind === "paragraph" ? "min-h-16" : cell ? "formatted-pdf-cell-field" : undefined}
        onChange={(next) => setRuns(key, next)}
        onFocus={() => setSelectedKey(key)}
        onKeyDown={(event) => fieldKeyDown(place, key, event)}
        onPaste={(event) => paste(place, key, event)}
      />
    );
  };

  return (
    <div className="formatted-pdf">
      <Card className="workspace-control-panel py-0">
        <CardContent className="workspace-control-panel-content grid gap-4 p-4">
          <div>
            <h2 className="workspace-active-command">{utility.title}</h2>
            <p className="workspace-panel-copy">{utility.blurb}</p>
          </div>
          <div className="formatted-pdf-layout">
            <section className="formatted-pdf-editor" aria-label="Document">
              <TooltipProvider>
                <div className="formatted-pdf-toolbar" role="toolbar" aria-label="Formatting">
                  <Label htmlFor="formatted-pdf-block-type" className="sr-only">Block type</Label>
                  <NativeSelect
                    id="formatted-pdf-block-type"
                    size="sm"
                    value={current.kind}
                    disabled={saving || fixedKind}
                    onChange={(event) => fromToolbar(event.target.value as FormatKeyId)}
                  >
                    {typeChoices.map((kind) => <NativeSelectOption key={kind} value={kind}>{kindLabels[kind]}</NativeSelectOption>)}
                    {fixedKind && <NativeSelectOption value={current.kind} disabled>{kindLabels[current.kind]}</NativeSelectOption>}
                  </NativeSelect>
                  <Separator orientation="vertical" className="formatted-pdf-toolbar-separator" />
                  {tool("bold", "Bold", (
                    <Toggle size="sm" aria-label="Bold" aria-keyshortcuts={formatKeyAria("bold")} className="formatted-pdf-bold" pressed={!richOff && emphasis.bold} disabled={richOff} onMouseDown={keepFocus} onPressedChange={() => fromToolbar("bold")}>B</Toggle>
                  ))}
                  {tool("italic", "Italic", (
                    <Toggle size="sm" aria-label="Italic" aria-keyshortcuts={formatKeyAria("italic")} className="formatted-pdf-italic" pressed={!richOff && emphasis.italic} disabled={richOff} onMouseDown={keepFocus} onPressedChange={() => fromToolbar("italic")}>I</Toggle>
                  ))}
                  <Separator orientation="vertical" className="formatted-pdf-toolbar-separator" />
                  {tool("bulletedList", "Bulleted list", (
                    <Toggle size="sm" aria-label="Bulleted list" aria-keyshortcuts={formatKeyAria("bulletedList")} pressed={current.kind === "bulletedList"} disabled={saving || fixedKind} onMouseDown={keepFocus} onPressedChange={() => fromToolbar("bulletedList")}><List /></Toggle>
                  ))}
                  {tool("numberedList", "Numbered list", (
                    <Toggle size="sm" aria-label="Numbered list" aria-keyshortcuts={formatKeyAria("numberedList")} pressed={current.kind === "numberedList"} disabled={saving || fixedKind} onMouseDown={keepFocus} onPressedChange={() => fromToolbar("numberedList")}><ListOrdered /></Toggle>
                  ))}
                  <Separator orientation="vertical" className="formatted-pdf-toolbar-separator" />
                  <Button variant="ghost" size="sm" disabled={saving} onMouseDown={keepFocus} onClick={insertTable}><Table />Table</Button>
                  <Button variant="ghost" size="sm" disabled={saving} onMouseDown={keepFocus} onClick={() => void addPickedImage()}><ImageIcon />Image…</Button>
                  {tool("pageBreak", "Page break", (
                    <Button variant="ghost" size="sm" aria-keyshortcuts={formatKeyAria("pageBreak")} disabled={saving} onMouseDown={keepFocus} onClick={() => fromToolbar("pageBreak")}><SeparatorHorizontal />Page break</Button>
                  ))}
                </div>
              </TooltipProvider>
              {missingBlock && (
                <Card className="formatted-pdf-alert border-destructive py-0" role="alert">
                  <CardContent className="formatted-pdf-alert-content">
                    <CircleX aria-hidden="true" className="text-destructive" />
                    <p>
                      <strong>The PDF wasn't saved: {missingName} is missing</strong>
                      <span>It was at {missing} when you added it. Nothing was written and your draft is kept. Choose the image again or remove its block, then save.</span>
                    </p>
                  </CardContent>
                </Card>
              )}
              <ol ref={list} className="formatted-pdf-blocks" aria-label="Document blocks, in order">
                {blocks.map((block, index) => {
                  const number = index + 1;
                  const name = kindLabels[block.kind];
                  const ListTag = block.kind === "numberedList" ? "ol" : "ul";
                  return (
                    <li
                      key={block.id}
                      className="formatted-pdf-block"
                      data-kind={block.kind}
                      data-selected={index === currentPlace.index}
                      data-missing={block === missingBlock || undefined}
                      aria-label={isList(block)
                        ? `Block ${number}, ${name}, ${block.items.length} ${block.items.length === 1 ? "item" : "items"}`
                        : isTable(block) ? `Block ${number}, ${name}, ${block.rows[0].length} ${block.rows[0].length === 1 ? "column" : "columns"}${block.headerRow ? ", header row" : ""}` : undefined}
                    >
                      <span className="formatted-pdf-gutter" aria-hidden="true">{gutterLabels[block.kind]}</span>
                      {block.kind === "pageBreak" ? (
                        <div
                          className="formatted-pdf-break"
                          role="group"
                          aria-label={`Block ${number}, ${name}`}
                          tabIndex={0}
                          data-field-key={block.id}
                          onFocus={() => { setSelectedKey(String(block.id)); lastSelection.current = { key: String(block.id), start: 0, end: 0 }; }}
                          onKeyDown={(event) => fieldKeyDown({ index, block, itemIndex: null }, String(block.id), event)}
                        >
                          {name}
                        </div>
                      ) : block.kind === "image" ? (
                        <div
                          className="formatted-pdf-image"
                          role="group"
                          aria-label={`Block ${number}, ${name}, ${fileName(block.path)}`}
                          tabIndex={0}
                          data-field-key={block.id}
                          onFocus={() => { setSelectedKey(String(block.id)); lastSelection.current = { key: String(block.id), start: 0, end: 0 }; }}
                          onKeyDown={(event) => { if (event.target === event.currentTarget) fieldKeyDown({ index, block, itemIndex: null }, String(block.id), event); }}
                        >
                          <span className="formatted-pdf-thumb" aria-hidden="true">{block === missingBlock && <ImageOff className="text-destructive" />}</span>
                          <span className="formatted-pdf-image-text">
                            <span className="formatted-pdf-image-name">{fileName(block.path)}</span>
                            {block === missingBlock
                              ? <span className="workspace-note text-destructive">Not found at {block.path}</span>
                              : <span className="workspace-note">Fits the page width. Read from your disk when you save.</span>}
                            {block === missingBlock && (
                              <span className="formatted-pdf-image-actions">
                                <Button variant="outline" size="sm" disabled={saving} onClick={() => void replaceImage(index, block)}>Choose image…</Button>
                                <Button variant="ghost" size="sm" disabled={saving} onClick={() => remove(index)}>Remove block</Button>
                              </span>
                            )}
                          </span>
                        </div>
                      ) : isList(block) ? (
                        <ListTag className="formatted-pdf-items" start={starts[index] ?? undefined}>
                          {block.items.map((entry, itemIndex) => (
                            <li key={entry.id}>
                              {field(block, index, String(entry.id), `Block ${number}, ${name}, item ${itemIndex + 1}`, entry.runs, false, itemIndex)}
                            </li>
                          ))}
                        </ListTag>
                      ) : isTable(block) ? (
                        <div className="formatted-pdf-table">
                          <table>
                            {block.headerRow && (
                              <thead>
                                <tr>
                                  {block.rows[0].map((cell, column) => (
                                    <th key={cell.id} scope="col">{field(block, index, String(cell.id), `Block ${number}, ${name}, row 1, column ${column + 1}`, cell.runs, true, column)}</th>
                                  ))}
                                </tr>
                              </thead>
                            )}
                            <tbody>
                              {block.rows.slice(block.headerRow ? 1 : 0).map((row, at) => {
                                const rowIndex = at + (block.headerRow ? 1 : 0);
                                return (
                                  <tr key={row[0].id}>
                                    {row.map((cell, column) => (
                                      <td key={cell.id}>{field(block, index, String(cell.id), `Block ${number}, ${name}, row ${rowIndex + 1}, column ${column + 1}`, cell.runs, true, rowIndex * row.length + column)}</td>
                                    ))}
                                  </tr>
                                );
                              })}
                            </tbody>
                          </table>
                          {/* The board shows the table's controls while the caret is in it. */}
                          {index === currentPlace.index && <div className="formatted-pdf-table-actions">
                            <div className="formatted-pdf-table-header">
                              <Checkbox id={`formatted-pdf-header-row-${block.id}`} checked={block.headerRow} disabled={saving} onCheckedChange={(checked) => setHeaderRow(index, checked === true)} />
                              <Label htmlFor={`formatted-pdf-header-row-${block.id}`}>Header row</Label>
                            </div>
                            <Button variant="ghost" size="sm" aria-label={`Add a row to block ${number}`} disabled={saving || block.rows.length >= MAX_TABLE_ROWS} onClick={() => growTable(index, block, "row")}><Plus />Add row</Button>
                            <Button variant="ghost" size="sm" aria-label={`Add a column to block ${number}`} disabled={saving || block.rows[0].length >= MAX_TABLE_COLUMNS} onClick={() => growTable(index, block, "column")}><Plus />Add column</Button>
                          </div>}
                        </div>
                      ) : (
                        field(block, index, String(block.id), `Block ${number}, ${name}`, block.runs, block.kind !== "paragraph", null)
                      )}
                      <div className="formatted-pdf-block-actions">
                        <Button variant="ghost" size="icon" aria-label={`Move block ${number} up`} aria-keyshortcuts={formatKeyAria("moveUp")} disabled={saving || index === 0} onClick={() => move(index, -1)}><ArrowUp /></Button>
                        <Button variant="ghost" size="icon" aria-label={`Move block ${number} down`} aria-keyshortcuts={formatKeyAria("moveDown")} disabled={saving || index === blocks.length - 1} onClick={() => move(index, 1)}><ArrowDown /></Button>
                        <Button variant="ghost" size="icon" aria-label={`Delete block ${number}`} disabled={saving} onClick={() => remove(index)}><Trash2 /></Button>
                      </div>
                    </li>
                  );
                })}
              </ol>
              <p id="formatted-pdf-typing-hint" className="workspace-note">
                Enter starts a new paragraph or list item, and Enter on an empty item ends the list. Shift+Enter adds a line break. Pasted text keeps its paragraphs. A table cell holds one line, and its blank rows and columns are left out. Drop an image from {isWindows() ? "File Explorer" : "Finder"} to add it where you drop.
              </p>
            </section>
            <aside className="formatted-pdf-settings" aria-label="Document settings">
              <FormattedPdfPreview
                preview={preview}
                updating={updating}
                error={previewError}
                pageName={`${pageSizes[page.pageSize].name} ${page.orientation}`}
                pageSize={previewSize}
              />
              <fieldset className="workspace-field" disabled={saving} aria-describedby="formatted-pdf-page-summary">
                <legend className="workspace-panel-label">Page</legend>
                <p id="formatted-pdf-page-summary" className="workspace-note">{pageSummary(page)}</p>
                <div className="workspace-field">
                  <Label htmlFor="formatted-pdf-page-size">Size</Label>
                  <NativeSelect id="formatted-pdf-page-size" value={page.pageSize} onChange={(event) => setPageSetting("pageSize", event.target.value as PageSize)}>
                    {(Object.keys(pageSizes) as PageSize[]).map((size) => <NativeSelectOption key={size} value={size}>{pageSizes[size].choice}</NativeSelectOption>)}
                  </NativeSelect>
                </div>
                <div className="workspace-field">
                  <Label id="formatted-pdf-orientation-label">Orientation</Label>
                  <ToggleGroup
                    type="single" variant="outline" size="sm" className="formatted-pdf-segments" aria-labelledby="formatted-pdf-orientation-label"
                    value={page.orientation} onValueChange={(value) => { if (value) setPageSetting("orientation", value as Orientation); }}
                  >
                    <ToggleGroupItem value="portrait">Portrait</ToggleGroupItem>
                    <ToggleGroupItem value="landscape">Landscape</ToggleGroupItem>
                  </ToggleGroup>
                </div>
                <div className="workspace-field">
                  <Label id="formatted-pdf-margins-label">Margins</Label>
                  <ToggleGroup
                    type="single" variant="outline" size="sm" className="formatted-pdf-segments" aria-labelledby="formatted-pdf-margins-label" aria-describedby="formatted-pdf-margins-hint"
                    value={page.margins} onValueChange={(value) => { if (value) setPageSetting("margins", value as Margins); }}
                  >
                    {(Object.keys(marginLabels) as Margins[]).map((margins) => <ToggleGroupItem key={margins} value={margins}>{marginLabels[margins].name}</ToggleGroupItem>)}
                  </ToggleGroup>
                  <p id="formatted-pdf-margins-hint" className="workspace-note">{marginLabels[page.margins].hint}</p>
                </div>
              </fieldset>
              <dl className="formatted-pdf-defaults">
                <dt className="workspace-panel-label">Font</dt>
                <dd className="workspace-note">Liberation Serif, 11 pt</dd>
              </dl>
              <fieldset className="workspace-field" disabled={saving} aria-describedby="formatted-pdf-band-summary">
                <legend className="workspace-panel-label">Header and footer</legend>
                <p id="formatted-pdf-band-summary" className="workspace-note">{bandSummary(page)}</p>
                <div className="formatted-pdf-pair">
                  <div className="workspace-field">
                    <Label htmlFor="formatted-pdf-header">Header text</Label>
                    <Input id="formatted-pdf-header" value={page.header} onChange={(event) => setPageSetting("header", event.target.value)} />
                  </div>
                  <div className="workspace-field">
                    <Label htmlFor="formatted-pdf-footer">Footer text</Label>
                    <Input id="formatted-pdf-footer" placeholder="None" value={page.footer} onChange={(event) => setPageSetting("footer", event.target.value)} />
                  </div>
                </div>
                <Label htmlFor="formatted-pdf-page-numbers" className="cursor-pointer">
                  <Checkbox id="formatted-pdf-page-numbers" checked={page.pageNumbers} onCheckedChange={(checked) => setPageSetting("pageNumbers", checked === true)} />
                  Page numbers at the bottom
                </Label>
              </fieldset>
              <fieldset className="workspace-field" disabled={saving}>
                <legend className="workspace-panel-label">Title and author</legend>
                <div className="workspace-field">
                  <Label htmlFor="formatted-pdf-title">Title</Label>
                  <Input id="formatted-pdf-title" aria-describedby="formatted-pdf-details-hint" value={title} onChange={(event) => setTitle(event.target.value)} />
                </div>
                <div className="workspace-field">
                  <Label htmlFor="formatted-pdf-author">Author</Label>
                  <Input id="formatted-pdf-author" aria-describedby="formatted-pdf-details-hint" value={author} onChange={(event) => setAuthor(event.target.value)} />
                </div>
                <p id="formatted-pdf-details-hint" className="workspace-note">Saved in the PDF's details. The title is also the suggested file name.</p>
              </fieldset>
            </aside>
          </div>
          <footer className="formatted-pdf-footer">
            {empty && <p id="formatted-pdf-blocker" className="workspace-note">Empty document. Add some text to save a PDF.</p>}
            {!empty && !missingBlock && estimate && (
              <p id="formatted-pdf-estimate" className="formatted-pdf-status">
                <strong>{estimate}</strong>
                <span className="workspace-note">Opens the save dialog. Nothing is replaced.</span>
              </p>
            )}
            {missingBlock && (
              <p id="formatted-pdf-status" className="formatted-pdf-status" role="status" aria-live="polite">
                <strong className="text-destructive">Save failed, nothing written</strong>
                <span className="workspace-note">Fix the image block, then save again</span>
              </p>
            )}
            <Button
              variant="default"
              aria-describedby={empty ? "formatted-pdf-blocker" : missingBlock ? "formatted-pdf-status" : estimate ? "formatted-pdf-estimate" : undefined}
              disabled={saving || empty}
              onClick={() => void save()}
              className="workspace-primary-action"
              {...shortcutTarget("run")}
            >
              Save PDF…
              <ShortcutHint id="run" />
            </Button>
          </footer>
        </CardContent>
      </Card>
      {(saving || saved) && (
        <ResultList
          results={saved ?? []}
          progress={{ completed: saved ? 1 : 0, total: 1 }}
          loading={saving}
          resultVerb={utility.resultVerb}
          resultNoun={utility.resultNoun}
          runningVerb={utility.runningVerb}
          live={null}
          namedByOutput
        />
      )}
      {saved && !saving && <Button variant="outline" className="formatted-pdf-new" onClick={startOver}>New document</Button>}
    </div>
  );
};
