import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Plus, X } from "lucide-react";
import type { ImagePreview, TiffCompression } from "../../contracts";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { fileName } from "../../components/intake";
import { ShortcutHint } from "../../components/ShortcutHint";
import { isWindows } from "../../shortcuts";
import { useTileOrdering } from "./useTileOrdering";

/** One file's pages, read once for its list entry. */
export type TiffSource =
  | { kind: "loading" }
  | { kind: "ready"; previews: (ImagePreview | null)[] }
  | { kind: "error"; message: string };

/** Split writes each file's pages separately; Combine joins every page into one TIFF. Never inferred from file count. */
export type TiffMode = "split" | "combine";

export interface TiffPagesState {
  mode: TiffMode;
  /** Each file's pages, by its list entry key. */
  sources: Record<string, TiffSource>;
  /** Page keys in the order the person set. A page not named here follows, in file order. */
  order: string[];
  /** Page keys left out of the combined TIFF. The files themselves don't change. */
  leftOut: string[];
  /** The selected page's key. */
  selected: string | null;
  /** How every written page is compressed, in both modes. The source compression isn't kept. */
  compression: TiffCompression;
}

/** One page in the combined order. */
export interface TiffPage {
  key: string;
  path: string;
  /** The page's index in its file, from 0. */
  page: number;
  /** The source file's letter: A for the first file, B for the second, and so on. */
  letter: string;
  preview: ImagePreview | null;
}

export const initialTiffPagesState: TiffPagesState = { mode: "split", sources: {}, order: [], leftOut: [], selected: null, compression: "lzw" };

const pageKey = (fileKey: string, page: number) => `${fileKey}\u0000${page}`;

/** A, B, … Z, then AA, AB and so on. */
export const sourceLetter = (index: number): string =>
  (index >= 26 ? sourceLetter(Math.floor(index / 26) - 1) : "") + String.fromCharCode(65 + (index % 26));

const filePages = (state: TiffPagesState, files: string[], keys: string[]): TiffPage[] =>
  keys.flatMap((fileKey, fileIndex) => {
    const source = state.sources[fileKey];
    if (source?.kind !== "ready") return [];
    return source.previews.map((preview, page) => ({ key: pageKey(fileKey, page), path: files[fileIndex], page, letter: sourceLetter(fileIndex), preview }));
  });

/** Every page that goes into the combined TIFF, in order: the person's order first, then new pages in file order. */
export const tiffPagesInOrder = (state: TiffPagesState, files: string[], keys: string[]): TiffPage[] => {
  const pages = filePages(state, files, keys);
  const byKey = new Map(pages.map((page) => [page.key, page]));
  const leftOut = new Set(state.leftOut);
  const ordered = new Set(state.order);
  return [
    ...state.order.flatMap((key) => {
      const page = byKey.get(key);
      return page && !leftOut.has(key) ? [page] : [];
    }),
    ...pages.filter((page) => !ordered.has(page.key) && !leftOut.has(page.key)),
  ];
};

/** Whether every current file's pages have been read, and at least one page goes into the TIFF. */
export const canCombineTiffPages = (state: TiffPagesState, files: string[], keys: string[]) =>
  keys.length > 0 && keys.every((key) => state.sources[key]?.kind === "ready") && tiffPagesInOrder(state, files, keys).length > 0;

const pageCount = (count: number) => `${count} ${count === 1 ? "page" : "pages"}`;
const pageName = (page: TiffPage) => `${fileName(page.path)}, page ${page.page + 1}`;

/**
 * The pages of every chosen TIFF, in the order they are joined. A page moves by dragging it or
 * with the modifier and an arrow key; Delete or Leave page out takes it out of the combined TIFF.
 */
export const TiffPageInspector = ({ files, fileKeys, removeFile, browse, supportsPreview, disabled, state, setState }: {
  files: string[];
  fileKeys: string[];
  removeFile: (index: number) => void;
  browse: () => void;
  supportsPreview: boolean;
  disabled: boolean;
  state: TiffPagesState;
  setState: (update: (current: TiffPagesState) => TiffPagesState) => void;
}) => {
  const requested = useRef(new Set<string>());

  // Reads each new file's pages once, and forgets files that left the list.
  useEffect(() => {
    const current = new Set(fileKeys);
    for (const key of requested.current) if (!current.has(key)) requested.current.delete(key);
    setState((latest) => {
      const sources = Object.fromEntries(Object.entries(latest.sources).filter(([key]) => current.has(key)));
      for (const key of fileKeys) sources[key] ??= { kind: "loading" };
      return { ...latest, sources };
    });
    fileKeys.forEach((key, index) => {
      if (requested.current.has(key)) return;
      requested.current.add(key);
      const settle = (source: TiffSource) => setState((latest) => key in latest.sources ? { ...latest, sources: { ...latest.sources, [key]: source } } : latest);
      invoke<(ImagePreview | null)[]>("inspect_tiff_pages", { request: { path: files[index] } })
        .then((previews) => settle({ kind: "ready", previews }))
        .catch((error) => settle({ kind: "error", message: String(error) }));
    });
  }, [files, fileKeys, setState]);

  // Pages are read again whenever the page grid returns, so no preview outlives it.
  useEffect(() => () => {
    requested.current.clear();
    setState((latest) => ({ ...latest, sources: {} }));
  }, [setState]);

  const allPages = filePages(state, files, fileKeys);
  const pages = tiffPagesInOrder(state, files, fileKeys);
  const total = allPages.length;
  const found = pages.findIndex((page) => page.key === state.selected);
  const selectedIndex = Math.max(found, 0);
  const keys = pages.map((page) => page.key);
  const select = (index: number) => setState((latest) => ({ ...latest, selected: keys[index] ?? null }));
  const move = (index: number, toIndex: number) => setState((latest) => {
    const order = [...keys];
    const [moved] = order.splice(index, 1);
    order.splice(toIndex, 0, moved);
    return { ...latest, order, selected: moved };
  });
  const leaveOut = (index: number) => setState((latest) => ({
    ...latest,
    leftOut: [...latest.leftOut, keys[index]],
    selected: keys[index + 1] ?? keys[index - 1] ?? null,
  }));
  const reset = () => setState((latest) => ({ ...latest, order: [], leftOut: [] }));
  const { drag, onKeyDown, itemProps, buttonProps } = useTileOrdering({ keys, selectedIndex, selectIndex: select, move, remove: leaveOut, disabled });

  const leftOut = allPages.filter((page) => state.leftOut.includes(page.key));
  const selected = pages[selectedIndex];
  const dragged = drag && pages[drag.index];
  const status = dragged
    ? `Dragging ${pageName(dragged)}.${drag.over === null ? "" : ` Drop it at position ${drag.over + 1}.`}`
    : [
        selected ? `Position ${selectedIndex + 1} of ${pages.length} selected: ${pageName(selected)}.` : "",
        leftOut.length > 0 ? `Left out: ${leftOut.map(pageName).join("; ")}.` : "",
      ].filter(Boolean).join(" ");
  const loading = fileKeys.some((key) => state.sources[key]?.kind === "loading");
  const modifier = isWindows() ? "Ctrl" : "⌘";

  return (
    <Card className="media-frame-order py-0" role="region" aria-label="Pages">
      <CardContent className="media-frame-order-content py-4">
        <div className="media-page-toolbar">
          <strong className="workspace-panel-label">
            {pages.length} of {pageCount(total)}, from {files.length === 1 ? "1 TIFF" : `${files.length} TIFFs`}
          </strong>
          <Button variant="outline" size="sm" onClick={browse} disabled={disabled}><Plus aria-hidden="true" />Add TIFFs<ShortcutHint id="open" /></Button>
          <Button variant="outline" size="sm" onClick={() => leaveOut(selectedIndex)} disabled={disabled || !selected}>Leave page out</Button>
          <Button variant="ghost" size="sm" onClick={reset} disabled={disabled || (state.order.length === 0 && state.leftOut.length === 0)}>Reset pages</Button>
        </div>
        <ul className="media-page-sources" role="list" aria-label="Source files">
          {files.map((path, index) => {
            const source = state.sources[fileKeys[index]];
            return (
              <li key={fileKeys[index]}>
                <Badge variant="secondary" aria-hidden="true">{sourceLetter(index)}</Badge>
                <span>{fileName(path)}</span>
                <span className="workspace-note">
                  {source?.kind === "ready" ? pageCount(source.previews.length) : source?.kind === "error" ? "Can't read" : "Reading pages"}
                </span>
                <Button variant="ghost" size="icon-sm" aria-label={`Remove ${fileName(path)}`} onClick={() => removeFile(index)} disabled={disabled}><X aria-hidden="true" /></Button>
              </li>
            );
          })}
        </ul>
        <p className="workspace-panel-copy">
          Pages are joined in this order. Drag a page, or select it and press {modifier}+Arrow to move it. Delete leaves a page out; the file isn't changed. Reset pages brings every page back in file order.
        </p>
        {loading && <p className="workspace-note" role="status">Reading TIFF pages on this device.</p>}
        {files.flatMap((path, index) => {
          const source = state.sources[fileKeys[index]];
          return source?.kind === "error"
            ? [<p key={fileKeys[index]} className="workspace-note" role="alert">The pages of {fileName(path)} could not be read. {source.message}</p>]
            : [];
        })}
        {pages.length === 0 ? (
          !loading && <p className="workspace-note">{total > 0 ? "Every page is left out. Reset pages brings them back." : "Select TIFF files to arrange their pages here."}</p>
        ) : (
          <ol className="media-frame-grid" role="listbox" aria-label="Pages, in order" aria-orientation="horizontal" onKeyDown={onKeyDown} data-dragging={drag ? "true" : undefined}>
            {pages.map((page, index) => {
              const isSelected = index === selectedIndex;
              const label = `Position ${index + 1}, ${fileName(page.path)} page ${page.page + 1}`;
              return (
                <li key={page.key} role="option" aria-selected={isSelected} aria-label={label} {...itemProps(index)} className="media-frame-tile">
                  <Button {...buttonProps(index)} variant={isSelected ? "secondary" : "ghost"} className="media-frame-tile-button" aria-label={label}>
                    <span className="media-frame-picture" aria-hidden="true">
                      {supportsPreview && page.preview && <img src={page.preview.dataUrl} alt="" draggable={false} />}
                    </span>
                    <span className="media-frame-meta" aria-hidden="true">
                      <span>{index + 1}</span>
                      <span className="media-page-source"><Badge variant="secondary">{page.letter}</Badge>p{page.page + 1}</span>
                    </span>
                  </Button>
                </li>
              );
            })}
          </ol>
        )}
        {status && <p className="workspace-note" role="status" aria-live="polite">{status}</p>}
      </CardContent>
    </Card>
  );
};
