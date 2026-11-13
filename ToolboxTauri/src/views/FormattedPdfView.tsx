import { useContext, useEffect, useRef, useState, type ClipboardEvent, type KeyboardEvent, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowDown, ArrowUp, List, ListOrdered, SeparatorHorizontal, Trash2 } from "lucide-react";
import type { JobOutcome, ToolDefinition } from "../contracts";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import { Separator } from "@/components/ui/separator";
import { Toggle } from "@/components/ui/toggle";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { ResultList } from "../components/ResultList";
import { RunStateContext } from "../components/ToolScaffold";
import { ShortcutHint } from "../components/ShortcutHint";
import { chooseCreatedDocumentSavePath } from "../components/saveStep";
import { failedOutcome, parseToolResult } from "../jobOutcome";
import { formatKeyAria, formatKeyLabel, matchFormatKey, shortcutTarget, type FormatKeyId } from "../shortcuts";
import {
  RichField, insertIntoRuns, normalizeRuns, plainRuns, runsText, selectionOffsets, setSelectionOffsets, sliceRuns, splitRunLines, type Run,
} from "./FormattedPdfRichField";

type TextKind = "heading1" | "heading2" | "heading3" | "paragraph";
type ListKind = "bulletedList" | "numberedList";
export type BlockKind = TextKind | ListKind | "pageBreak";

type Item = { id: number; runs: Run[] };
type Block =
  | { id: number; kind: TextKind; runs: Run[] }
  | { id: number; kind: ListKind; items: Item[] }
  | { id: number; kind: "pageBreak" };

const kindLabels: Record<BlockKind, string> = {
  heading1: "Heading 1", heading2: "Heading 2", heading3: "Heading 3", paragraph: "Paragraph",
  bulletedList: "Bulleted list", numberedList: "Numbered list", pageBreak: "Page break",
};
const gutterLabels: Record<BlockKind, string> = {
  heading1: "H1", heading2: "H2", heading3: "H3", paragraph: "Text", bulletedList: "List", numberedList: "List", pageBreak: "Break",
};
const typeChoices: readonly (Exclude<BlockKind, "pageBreak"> & FormatKeyId)[] = ["paragraph", "heading1", "heading2", "heading3", "bulletedList", "numberedList"];

/** The defaults this first version always uses; page setup and the font picker come later. */
const PAGE = { pageSize: "a4", margins: "normal" } as const;
const FONT = { family: "Liberation Serif", size: 11 } as const;

let lastId = 0;
const nextId = () => ++lastId;
const textBlock = (kind: TextKind = "paragraph", runs: Run[] = []): Block => ({ id: nextId(), kind, runs: kind === "paragraph" ? runs : plainRuns(runsText(runs)) });
const item = (runs: Run[] = []): Item => ({ id: nextId(), runs });
const pageBreak = (): Block => ({ id: nextId(), kind: "pageBreak" });

const isList = (block: Block): block is Extract<Block, { kind: ListKind }> => block.kind === "bulletedList" || block.kind === "numberedList";
const isText = (block: Block): block is Extract<Block, { kind: TextKind }> => block.kind !== "pageBreak" && !isList(block);
const blank = (runs: readonly Run[]) => runsText(runs).trim() === "";
const blockIsBlank = (block: Block) => block.kind === "pageBreak" || (isList(block) ? block.items.every((entry) => blank(entry.runs)) : blank(block.runs));
const firstKey = (block: Block) => String(isList(block) ? block.items[0].id : block.id);

/** Where a field sits: the block, and the list item for list blocks. */
type Place = { index: number; block: Block; itemIndex: number | null };
const locate = (blocks: readonly Block[], key: string | null): Place | null => {
  if (key === null) return null;
  for (const [index, block] of blocks.entries()) {
    if (String(block.id) === key && !isList(block)) return { index, block, itemIndex: null };
    if (isList(block)) {
      const itemIndex = block.items.findIndex((entry) => String(entry.id) === key);
      if (itemIndex !== -1) return { index, block, itemIndex };
    }
  }
  return null;
};
const placeKey = ({ block, itemIndex }: Place) => String(isList(block) ? block.items[itemIndex ?? 0].id : block.id);
const placeRuns = (place: Place): Run[] =>
  place.block.kind === "pageBreak" ? [] : isList(place.block) ? place.block.items[place.itemIndex ?? 0].runs : place.block.runs;

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
  | { kind: "pageBreak" };

/** What create_formatted_pdf receives: blank blocks and items are left out, headings carry their text only. */
const requestBlocks = (blocks: readonly Block[]) => blocks.flatMap((block): RequestBlock[] => {
  if (block.kind === "pageBreak") return [{ kind: "pageBreak" }];
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

export const FormattedPdfView = ({ utility }: { utility: ToolDefinition }) => {
  const [blocks, setBlocks] = useState<Block[]>(() => [textBlock()]);
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [emphasis, setEmphasis] = useState({ bold: false, italic: false });
  const [title, setTitle] = useState("");
  const [author, setAuthor] = useState("");
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState<JobOutcome[] | null>(null);
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
  const starts = listStarts(blocks);

  const replace = (index: number, replacement: Block[], focus: Selection) => {
    focusNext.current = focus;
    setBlocks((all) => [...all.slice(0, index), ...replacement, ...all.slice(index + 1)]);
  };

  const setRuns = (key: string, runs: Run[]) => setBlocks((all) => all.map((block) => {
    if (String(block.id) === key && isText(block)) return { ...block, runs };
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
    // Text on one line is typed in by the browser, so it keeps the style at the caret and the browser's undo.
    if (!/[\r\n]/.test(text)) { document.execCommand("insertText", false, text); return; }
    const parts = pastedParts(text, place.itemIndex !== null);
    if (parts.length > 0) insertParts(place, key, offsets.start, offsets.end, parts);
  };

  /** Applies a block type: headings and paragraphs keep the caret, lists take one item per line and give it back. */
  const changeKind = (place: Place, start: number, end: number, kind: Exclude<BlockKind, "pageBreak">) => {
    const { index, block, itemIndex } = place;
    if (block.kind === "pageBreak" || block.kind === kind) return;
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
    if (block.kind === "pageBreak") { replace(index, [block, added], { key: String(added.id), start: 0, end: 0 }); return; }
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
    if (place.block.kind === "pageBreak") {
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

  const save = async () => {
    if (empty || saving) return;
    const name = `${title.trim() || "Untitled"}.pdf`;
    const outputPath = await chooseCreatedDocumentSavePath({ title: "Save the PDF", extension: "pdf" }, name, "A4, Liberation Serif 11 pt");
    if (!outputPath) return;
    setSaving(true);
    reportRunning?.(true);
    try {
      const request = { document: { title: title.trim(), author: author.trim(), blocks: requestBlocks(blocks), page: PAGE, font: FONT }, outputPath };
      setSaved(parseToolResult(await invoke("create_formatted_pdf", { request })));
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
    setSaved(null);
    lastSelection.current = null;
    focusNext.current = { key: String(first.id), start: 0, end: 0 };
  };

  const richOff = saving || (current.kind !== "paragraph" && !isList(current));
  const keepFocus = (event: { preventDefault: () => void }) => event.preventDefault();
  const tool = (id: FormatKeyId, label: string, control: ReactNode) => (
    <Tooltip>
      <TooltipTrigger asChild>{control}</TooltipTrigger>
      <TooltipContent>{label} <kbd className="opacity-70">{formatKeyLabel(id)}</kbd></TooltipContent>
    </Tooltip>
  );

  const field = (block: Block, index: number, key: string, label: string, runs: Run[], plain: boolean, itemIndex: number | null) => {
    const place = { index, block, itemIndex };
    return (
      <RichField
        fieldKey={key}
        label={label}
        runs={runs}
        plain={plain}
        disabled={saving}
        describedBy="formatted-pdf-typing-hint"
        className={block.kind === "paragraph" ? "min-h-16" : undefined}
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
                    disabled={saving || current.kind === "pageBreak"}
                    onChange={(event) => fromToolbar(event.target.value as FormatKeyId)}
                  >
                    {typeChoices.map((kind) => <NativeSelectOption key={kind} value={kind}>{kindLabels[kind]}</NativeSelectOption>)}
                    {current.kind === "pageBreak" && <NativeSelectOption value="pageBreak" disabled>{kindLabels.pageBreak}</NativeSelectOption>}
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
                    <Toggle size="sm" aria-label="Bulleted list" aria-keyshortcuts={formatKeyAria("bulletedList")} pressed={current.kind === "bulletedList"} disabled={saving || current.kind === "pageBreak"} onMouseDown={keepFocus} onPressedChange={() => fromToolbar("bulletedList")}><List /></Toggle>
                  ))}
                  {tool("numberedList", "Numbered list", (
                    <Toggle size="sm" aria-label="Numbered list" aria-keyshortcuts={formatKeyAria("numberedList")} pressed={current.kind === "numberedList"} disabled={saving || current.kind === "pageBreak"} onMouseDown={keepFocus} onPressedChange={() => fromToolbar("numberedList")}><ListOrdered /></Toggle>
                  ))}
                  <Separator orientation="vertical" className="formatted-pdf-toolbar-separator" />
                  {tool("pageBreak", "Page break", (
                    <Button variant="ghost" size="sm" aria-keyshortcuts={formatKeyAria("pageBreak")} disabled={saving} onMouseDown={keepFocus} onClick={() => fromToolbar("pageBreak")}><SeparatorHorizontal />Page break</Button>
                  ))}
                </div>
              </TooltipProvider>
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
                      aria-label={isList(block) ? `Block ${number}, ${name}, ${block.items.length} ${block.items.length === 1 ? "item" : "items"}` : undefined}
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
                      ) : isList(block) ? (
                        <ListTag className="formatted-pdf-items" start={starts[index] ?? undefined}>
                          {block.items.map((entry, itemIndex) => (
                            <li key={entry.id}>
                              {field(block, index, String(entry.id), `Block ${number}, ${name}, item ${itemIndex + 1}`, entry.runs, false, itemIndex)}
                            </li>
                          ))}
                        </ListTag>
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
                Enter starts a new paragraph or list item, and Enter on an empty item ends the list. Shift+Enter adds a line break. Pasted text keeps its paragraphs.
              </p>
            </section>
            <aside className="formatted-pdf-settings" aria-label="Document settings">
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
              <dl className="formatted-pdf-defaults">
                <dt className="workspace-panel-label">Page</dt>
                <dd className="workspace-note">A4, portrait, normal margins</dd>
                <dt className="workspace-panel-label">Font</dt>
                <dd className="workspace-note">Liberation Serif, 11 pt</dd>
              </dl>
            </aside>
          </div>
          <footer className="formatted-pdf-footer">
            {empty && <p id="formatted-pdf-blocker" className="workspace-note">Empty document. Add some text to save a PDF.</p>}
            <Button
              variant="default"
              aria-describedby={empty ? "formatted-pdf-blocker" : undefined}
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
