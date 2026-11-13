import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { NativeSelect } from "@/components/ui/native-select";
import type { PdfDocument, PdfPage } from "../pdf-editor/contracts";
import { isWindows } from "../../shortcuts";
import {
  describePage, keptRect, keptSize, moveRect, nudgeRect, paperName, rectBetween, resizeRect, sameRect, showLength, showSize, sizeKey,
  type Handle, type Margins, type PageSize, type Rect, type Unit,
} from "./geometry";

/** Page sizes for each PDF, read without rendering. Undefined while loading, "error" when unreadable. */
export type PageSizes = ReadonlyMap<string, readonly PageSize[] | "error">;

export const usePageSizes = (files: readonly string[]): PageSizes => {
  const [sizes, setSizes] = useState<ReadonlyMap<string, readonly PageSize[] | "error">>(() => new Map());
  const requested = useRef(new Set<string>());
  useEffect(() => {
    for (const path of files) {
      if (requested.current.has(path)) continue;
      requested.current.add(path);
      void invoke<PdfDocument>("inspect_pdf_scene", { request: { path } })
        .then((document) => document.pages.map((page) => ({ width: page.width, height: page.height })))
        .catch(() => "error" as const)
        .then((result) => setSizes((current) => new Map(current).set(path, result)));
    }
  }, [files]);
  return sizes;
};

/** The rendered image of one page, reusing the PDF editor's page preview. */
const usePagePreview = (path: string | undefined, pageIndex: number) => {
  const key = path === undefined ? null : `${path}#${pageIndex}`;
  const [preview, setPreview] = useState<{ key: string; image: string | null } | null>(null);
  useEffect(() => {
    if (path === undefined || key === null) return;
    let current = true;
    void invoke<PdfPage>("inspect_pdf_scene_page", { request: { path, pageIndex } })
      .then((page) => page.preview ?? null, () => null)
      .then((image) => { if (current) setPreview({ key, image }); });
    return () => { current = false; };
  }, [key, path, pageIndex]);
  return preview?.key === key ? preview.image : null;
};

export type CropTarget = { rect: Rect } | { margins: Margins };

const fileName = (path: string) => path.split(/[\\/]/).pop() ?? path;
const percent = (fraction: number) => `${fraction * 100}%`;
const boxStyle = (rect: Rect) => ({ left: percent(rect.left), top: percent(rect.top), width: percent(rect.width), height: percent(rect.height) });

const handles: ReadonlyArray<{ handle: Handle; x: number; y: number }> = [
  { handle: "nw", x: 0, y: 0 }, { handle: "n", x: 50, y: 0 }, { handle: "ne", x: 100, y: 0 }, { handle: "e", x: 100, y: 50 },
  { handle: "se", x: 100, y: 100 }, { handle: "s", x: 50, y: 100 }, { handle: "sw", x: 0, y: 100 }, { handle: "w", x: 0, y: 50 },
];

/** Shade over the parts of the page outside the kept area. */
const Shade = ({ rect }: { rect: Rect }) => {
  const right = rect.left + rect.width;
  const bottom = rect.top + rect.height;
  const bands: Rect[] = [
    { left: 0, top: 0, width: 1, height: rect.top },
    { left: 0, top: bottom, width: 1, height: 1 - bottom },
    { left: 0, top: rect.top, width: rect.left, height: rect.height },
    { left: right, top: rect.top, width: 1 - right, height: rect.height },
  ];
  return <>{bands.filter((band) => band.width > 0.001 && band.height > 0.001).map((band, index) => <div key={index} className="crop-shade" style={boxStyle(band)} />)}</>;
};

type Drag =
  | { kind: "draw"; start: Rect; origin: { x: number; y: number } }
  | { kind: "move"; start: Rect; origin: { x: number; y: number } }
  | { kind: "resize"; start: Rect; handle: Handle };

const STAGE = { width: 230, height: 238 };
const MINI = { width: 112, height: 98, pointsPerPixel: 25.4 / 72 / 0.32 };

export const CropPreview = ({
  files, selectedFileIndex, selectFile, sizes, pageIndex, onPageChange, target, invalidCaption, unit, editable, history, onRect, onRecord,
}: {
  files: readonly string[];
  selectedFileIndex: number;
  selectFile: (index: number) => void;
  sizes: PageSizes;
  pageIndex: number;
  onPageChange: (index: number) => void;
  /** The crop to show, or null while it can't be shown. */
  target: CropTarget | null;
  invalidCaption: string;
  unit: Unit;
  /** Whether the rectangle can be drawn, dragged and nudged here. */
  editable: boolean;
  history: { canUndo: boolean; canReset: boolean; onUndo: () => void; onReset: () => void } | null;
  /** Shows a rectangle while it changes. */
  onRect: (rect: Rect) => void;
  /** Records the rectangle a finished change started from, so Undo can bring it back. */
  onRecord: (previous: Rect) => void;
}) => {
  const path = files[selectedFileIndex];
  const pages = path === undefined ? undefined : sizes.get(path);
  const page = Array.isArray(pages) ? pages[pageIndex] : undefined;
  const image = usePagePreview(page ? path : undefined, pageIndex);
  const [drag, setDrag] = useState<Drag | null>(null);
  const stage = useRef<HTMLDivElement>(null);
  const rect = target && page ? keptRect(page, target) : null;
  const kept = target && page ? keptSize(page, target) : null;
  const pageCount = Array.isArray(pages) ? pages.length : 0;

  const pointAt = (event: React.PointerEvent) => {
    const bounds = stage.current!.getBoundingClientRect();
    return { x: (event.clientX - bounds.left) / bounds.width, y: (event.clientY - bounds.top) / bounds.height };
  };

  const onPointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    if (!editable || !target || !("rect" in target) || event.button !== 0) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    const point = pointAt(event);
    const handle = (event.target as HTMLElement).dataset.handle as Handle | undefined;
    const start = target.rect;
    const inside = point.x >= start.left && point.x <= start.left + start.width && point.y >= start.top && point.y <= start.top + start.height;
    setDrag(handle ? { kind: "resize", start, handle } : inside ? { kind: "move", start, origin: point } : { kind: "draw", start, origin: point });
  };
  const next = (event: React.PointerEvent, current: Drag) => {
    const point = pointAt(event);
    if (current.kind === "resize") return resizeRect(current.start, current.handle, point);
    if (current.kind === "move") return moveRect(current.start, point.x - current.origin.x, point.y - current.origin.y);
    return rectBetween(current.origin, point);
  };
  const onPointerMove = (event: React.PointerEvent) => { if (drag) onRect(next(event, drag)); };
  const onPointerUp = (event: React.PointerEvent) => {
    if (!drag) return;
    const result = next(event, drag);
    setDrag(null);
    // A click without a drag draws nothing, so it keeps the rectangle it started with.
    if (drag.kind === "draw" && (result.width < 0.01 || result.height < 0.01)) { onRect(drag.start); return; }
    onRect(result);
    if (!sameRect(result, drag.start)) onRecord(drag.start);
  };
  const onPointerCancel = () => { if (drag) onRect(drag.start); setDrag(null); };
  const onKeyDown = (event: React.KeyboardEvent) => {
    if (!editable || !page || !target || !("rect" in target) || event.metaKey || event.ctrlKey || event.altKey) return;
    const nudged = nudgeRect(target.rect, event.key, event.shiftKey, page);
    if (!nudged) return;
    event.preventDefault();
    if (sameRect(nudged, target.rect)) return;
    onRect(nudged);
    onRecord(target.rect);
  };

  const scale = page ? Math.min(STAGE.width / page.width, STAGE.height / page.height) : 0;
  const pageLabel = page ? paperName(page) ?? showSize(page, unit) : "";
  const alt = page && kept ? `Page ${pageIndex + 1}, ${pageLabel}. The kept area is ${showLength(kept.width, unit)} by ${showLength(kept.height, unit)} ${unit}.` : `Page ${pageIndex + 1}`;
  const caption = !page ? null : !target ? invalidCaption : kept ? `${describePage(page, unit)}. Keeps ${showSize(kept, unit)}.` : `${describePage(page, unit)}. No area left. This PDF will fail.`;
  const minis = page ? otherSizes(files, sizes, sizeKey(page)) : [];
  const miniScale = Math.min(1 / MINI.pointsPerPixel, ...minis.map((mini) => Math.min(MINI.width / mini.page.width, MINI.height / mini.page.height)));
  const undoKey = isWindows() ? "Ctrl+Z" : "⌘Z";

  return (
    <Card className="crop-preview py-0" role="region" aria-label="Preview">
      <CardContent className="p-0">
        <div className="crop-preview-header">
          <span className="crop-preview-label">Preview</span>
          <NativeSelect
            size="sm"
            aria-label="PDF to preview"
            className="crop-preview-file"
            value={String(selectedFileIndex)}
            onChange={(event) => { selectFile(Number(event.target.value)); onPageChange(0); }}
          >
            {files.map((file, index) => <option key={`${file}-${index}`} value={index}>{fileName(file)}</option>)}
          </NativeSelect>
          <span className="crop-preview-pager">
            <Button variant="ghost" size="icon-sm" aria-label="Previous page in range" disabled={pageIndex <= 0} onClick={() => onPageChange(pageIndex - 1)}>
              <ChevronLeft aria-hidden="true" />
            </Button>
            <span className="workspace-note" aria-live="polite">{pageCount ? `Page ${pageIndex + 1} of ${pageCount}` : ""}</span>
            <Button variant="ghost" size="icon-sm" aria-label="Next page in range" disabled={pageIndex >= pageCount - 1} onClick={() => onPageChange(pageIndex + 1)}>
              <ChevronRight aria-hidden="true" />
            </Button>
          </span>
          <span className="crop-preview-spacer" />
          {history && (
            <>
              <Button variant="ghost" size="sm" disabled={!history.canUndo} onClick={history.onUndo} aria-keyshortcuts={isWindows() ? "Control+Z" : "Meta+Z"}>
                Undo<kbd aria-hidden="true" className="shortcut-hint">{undoKey}</kbd>
              </Button>
              <Button variant="ghost" size="sm" disabled={!history.canReset} onClick={history.onReset}>Reset crop</Button>
            </>
          )}
        </div>
        <div className="crop-preview-body">
          <figure className="crop-preview-figure">
            <div className="crop-preview-well">
              {pages === "error" ? (
                <span className="workspace-note">This PDF can't be previewed.</span>
              ) : !page ? (
                <span className="workspace-note">Loading the preview…</span>
              ) : (
                <div
                  ref={stage}
                  className={`crop-stage${editable ? " crop-stage-editable" : ""}${drag ? " crop-stage-dragging" : ""}`}
                  style={{ width: page.width * scale, height: page.height * scale }}
                  tabIndex={editable ? 0 : -1}
                  role="group"
                  aria-label="Crop rectangle"
                  aria-describedby="crop-stage-alt"
                  onPointerDown={onPointerDown}
                  onPointerMove={onPointerMove}
                  onPointerUp={onPointerUp}
                  onPointerCancel={onPointerCancel}
                  onKeyDown={onKeyDown}
                >
                  <div role="img" id="crop-stage-alt" aria-label={alt} className="crop-page">
                    {image && <img src={image} alt="" draggable={false} />}
                  </div>
                  {rect ? (
                    <>
                      <Shade rect={rect} />
                      <div className="crop-frame" data-testid="crop-frame" style={boxStyle(rect)}>
                        {editable && handles.map(({ handle, x, y }) => (
                          <span key={handle} className={`crop-handle crop-handle-${handle}`} data-handle={handle} style={{ left: `${x}%`, top: `${y}%` }} />
                        ))}
                      </div>
                      {drag && kept && <span className="crop-chip" style={{ left: percent(rect.left + rect.width / 2), top: percent(rect.top + rect.height) }}>{showSize(kept, unit)}</span>}
                    </>
                  ) : target && <div className="crop-shade crop-shade-full" style={boxStyle({ left: 0, top: 0, width: 1, height: 1 })} />}
                </div>
              )}
            </div>
            {caption && <figcaption className={`workspace-note${!target || !kept ? " workspace-note-error" : ""}`} data-testid="crop-preview-caption">{caption}</figcaption>}
          </figure>
          {minis.length > 0 && (
            <div className="crop-minis">
              <span className="crop-preview-label">On other page sizes</span>
              <div className="crop-minis-row">
                {minis.map((mini) => {
                  const miniKept = target ? keptSize(mini.page, target) : null;
                  const miniRect = target ? keptRect(mini.page, target) : null;
                  const name = paperName(mini.page) ?? showSize(mini.page, unit);
                  const sub = !target ? name : miniKept ? `${name}. Keeps ${showSize(miniKept, unit)}` : `${name}. No area left. This PDF will fail.`;
                  return (
                    <figure key={mini.title} className="crop-mini" data-testid="crop-mini">
                      <div className="crop-mini-stage">
                        <div className={`crop-stage crop-stage-mini${target && !miniKept ? " crop-stage-empty" : ""}`} role="img" aria-label={`${mini.title}: ${sub}`} style={{ width: mini.page.width * miniScale, height: mini.page.height * miniScale }}>
                          <div className="crop-page" />
                          {miniRect ? <><Shade rect={miniRect} /><div className="crop-frame" style={boxStyle(miniRect)} /></> : target && <div className="crop-shade crop-shade-full" style={boxStyle({ left: 0, top: 0, width: 1, height: 1 })} />}
                        </div>
                      </div>
                      <figcaption>
                        <strong>{mini.title}</strong>
                        <span className={target && !miniKept ? "workspace-note-error" : undefined}>{sub}</span>
                      </figcaption>
                    </figure>
                  );
                })}
              </div>
              <p className="workspace-note">
                {target && "margins" in target
                  ? "Margins trim the same lengths from every page, so small pages lose more of their area."
                  : "The rectangle keeps its place relative to each page, so it lands in the same spot on any size or orientation."}
              </p>
            </div>
          )}
        </div>
      </CardContent>
    </Card>
  );
};

/** The first page of each other size in the batch, in list order, for the small previews. */
const otherSizes = (files: readonly string[], sizes: PageSizes, shown: string) => {
  const seen = new Set([shown]);
  const minis: Array<{ title: string; page: PageSize }> = [];
  for (const path of files) {
    const pages = sizes.get(path);
    if (!Array.isArray(pages)) continue;
    pages.forEach((page, index) => {
      const key = sizeKey(page);
      if (seen.has(key) || minis.length >= 3) return;
      seen.add(key);
      minis.push({ title: index === 0 ? fileName(path) : `${fileName(path)}, page ${index + 1}`, page });
    });
  }
  return minis;
};
