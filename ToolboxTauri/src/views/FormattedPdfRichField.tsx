import { useEffect, useRef, type ClipboardEvent, type KeyboardEvent } from "react";
import { cn } from "cn";

/** A stretch of text in one style, as create_formatted_pdf takes it. */
export type Run = { text: string; bold: boolean; italic: boolean };
type RunStyle = Pick<Run, "bold" | "italic">;

const PLAIN: RunStyle = { bold: false, italic: false };

/** Runs with empty runs dropped and neighbours in the same style merged, so equal text always has equal runs. */
export const normalizeRuns = (runs: readonly Run[]): Run[] => {
  const merged: Run[] = [];
  for (const run of runs) {
    if (run.text === "") continue;
    const last = merged[merged.length - 1];
    if (last && last.bold === run.bold && last.italic === run.italic) merged[merged.length - 1] = { ...last, text: last.text + run.text };
    else merged.push({ text: run.text, bold: run.bold, italic: run.italic });
  }
  return merged;
};

export const runsText = (runs: readonly Run[]) => runs.map((run) => run.text).join("");
export const plainRuns = (text: string): Run[] => normalizeRuns([{ text, ...PLAIN }]);
export const sameRuns = (a: readonly Run[], b: readonly Run[]) => JSON.stringify(normalizeRuns(a)) === JSON.stringify(normalizeRuns(b));

/** The runs covering characters `start` to `end` of the text. */
export const sliceRuns = (runs: readonly Run[], start: number, end = Number.POSITIVE_INFINITY): Run[] => {
  const sliced: Run[] = [];
  let at = 0;
  for (const run of runs) {
    const from = Math.max(start - at, 0);
    const to = Math.min(end - at, run.text.length);
    if (to > from) sliced.push({ ...run, text: run.text.slice(from, to) });
    at += run.text.length;
  }
  return normalizeRuns(sliced);
};

/** Text typed or pasted at `offset` takes the style of the character before it, as a text editor does. */
export const insertIntoRuns = (runs: readonly Run[], offset: number, text: string): Run[] => {
  const before = sliceRuns(runs, 0, offset);
  const style = before[before.length - 1] ?? sliceRuns(runs, offset)[0] ?? PLAIN;
  return normalizeRuns([...before, { text, bold: style.bold, italic: style.italic }, ...sliceRuns(runs, offset)]);
};

/** Splits runs at each line break, for turning lines into list items. */
export const splitRunLines = (runs: readonly Run[]): Run[][] => {
  const lines: Run[][] = [];
  let start = 0;
  const text = runsText(runs);
  for (let index = text.indexOf("\n"); index !== -1; index = text.indexOf("\n", start)) {
    lines.push(sliceRuns(runs, start, index));
    start = index + 1;
  }
  lines.push(sliceRuns(runs, start));
  return lines;
};

const SENTINEL = "data-line-end";

/** The runs a field's DOM shows. Style comes from what the browser renders, so <b>, <strong> and styled spans all count. */
function serialize(root: HTMLElement, plain: boolean): Run[] {
  const runs: Run[] = [];
  const walk = (node: Node) => {
    node.childNodes.forEach((child) => {
      if (child.nodeType === Node.TEXT_NODE) {
        const text = (child.textContent ?? "").replace(/ /g, " ");
        if (plain || !child.parentElement) { runs.push({ text, ...PLAIN }); return; }
        const style = getComputedStyle(child.parentElement);
        runs.push({ text, bold: Number(style.fontWeight) >= 600, italic: style.fontStyle !== "normal" });
      } else if (child instanceof HTMLElement && child.tagName !== "BR") walk(child);
    });
  };
  walk(root);
  return normalizeRuns(runs);
}

function render(root: HTMLElement, runs: readonly Run[], plain: boolean) {
  root.replaceChildren();
  for (const run of runs) {
    if (plain || (!run.bold && !run.italic)) { root.append(run.text); continue; }
    let element: HTMLElement = document.createElement(run.italic ? "i" : "b");
    element.textContent = run.text;
    if (run.bold && run.italic) {
      const bold = document.createElement("b");
      bold.append(element);
      element = bold;
    }
    root.append(element);
  }
  // A line break at the very end only shows as a new line when something follows it.
  if (runsText(runs).endsWith("\n")) {
    const end = document.createElement("br");
    end.setAttribute(SENTINEL, "");
    root.append(end);
  }
}

/** Character offsets of the selection inside the field, counting text the way serialize does. */
export function selectionOffsets(root: HTMLElement): { start: number; end: number } | null {
  const selection = window.getSelection();
  if (!selection || selection.rangeCount === 0) return null;
  const range = selection.getRangeAt(0);
  if (!root.contains(range.startContainer) || !root.contains(range.endContainer)) return null;
  const offset = (node: Node, at: number) => {
    const before = document.createRange();
    before.setStart(root, 0);
    before.setEnd(node, at);
    return before.toString().length;
  };
  return { start: offset(range.startContainer, range.startOffset), end: offset(range.endContainer, range.endOffset) };
}

export function setSelectionOffsets(root: HTMLElement, start: number, end = start) {
  const point = (target: number): [Node, number] => {
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    let at = 0;
    let last: Text | null = null;
    for (let node = walker.nextNode() as Text | null; node; node = walker.nextNode() as Text | null) {
      if (target <= at + node.length) return [node, target - at];
      at += node.length;
      last = node;
    }
    return last ? [last, last.length] : [root, 0];
  };
  const range = document.createRange();
  range.setStart(...point(start));
  range.setEnd(...point(end));
  const selection = window.getSelection();
  selection?.removeAllRanges();
  selection?.addRange(range);
}

interface RichFieldProps {
  fieldKey: string;
  label: string;
  runs: readonly Run[];
  /** Headings keep their text only: bold and italic do not apply to them. */
  plain: boolean;
  disabled: boolean;
  describedBy?: string;
  className?: string;
  onChange: (runs: Run[]) => void;
  onFocus: () => void;
  onKeyDown: (event: KeyboardEvent<HTMLDivElement>) => void;
  onPaste: (event: ClipboardEvent<HTMLDivElement>) => void;
}

/**
 * A block's text with bold and italic runs. The DOM is left to the browser while it matches the runs, so typing,
 * IME composition and the browser's own undo keep working; it is redrawn only when the runs change from outside.
 */
export const RichField = ({ fieldKey, label, runs, plain, disabled, describedBy, className, onChange, onFocus, onKeyDown, onPaste }: RichFieldProps) => {
    const root = useRef<HTMLDivElement>(null);
    useEffect(() => {
      const field = root.current;
      if (field && !sameRuns(serialize(field, plain), runs)) render(field, runs, plain);
    }, [runs, plain]);
    const emit = () => {
      const field = root.current;
      if (!field) return;
      const next = serialize(field, plain);
      // An emptied field drops the <br> browsers leave behind, so the placeholder shows again.
      if (next.length === 0 && field.childNodes.length > 0) field.replaceChildren();
      onChange(next);
    };
    return (
      <div
        ref={root}
        role="textbox"
        aria-label={label}
        aria-multiline="true"
        aria-describedby={describedBy}
        aria-disabled={disabled || undefined}
        contentEditable={!disabled}
        suppressContentEditableWarning
        tabIndex={disabled ? -1 : 0}
        data-field-key={fieldKey}
        data-plain={plain}
        data-placeholder="Type or paste text here"
        className={cn(
          "min-h-9 w-full min-w-0 rounded-md border border-input bg-transparent px-3 py-1.5 text-base leading-6 break-words whitespace-pre-wrap shadow-xs outline-none md:text-sm dark:bg-input/30",
          "focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 aria-disabled:cursor-not-allowed aria-disabled:opacity-50",
          "empty:before:text-muted-foreground empty:before:content-[attr(data-placeholder)]",
          className,
        )}
        onInput={emit}
        onFocus={onFocus}
        onKeyDown={onKeyDown}
        onPaste={onPaste}
        onDrop={(event) => event.preventDefault()}
      />
    );
};
