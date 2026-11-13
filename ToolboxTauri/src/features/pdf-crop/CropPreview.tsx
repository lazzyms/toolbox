import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { NativeSelect } from "@/components/ui/native-select";
import type { PdfDocument, PdfPage } from "../pdf-editor/contracts";
import { isWindows } from "../../shortcuts";
import {
  PAPER_TOLERANCE, cutsOff, describePage, keptRect, keptSize, moveRect, nudgeRect, paperName, placePage, rectBetween, resizeRect, sameRect, showLength, showSize, sizeKey,
  type Handle, type Margins, type Orientation, type PagePlacement, type PageSize, type Placement, type Rect, type Unit,
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

/** The parts of `outer` outside `inner`: full-width bands above and below, then the sides between them. */
const bandsOutside = (outer: Rect, inner: Rect): Rect[] => {
  const left = Math.max(outer.left, inner.left);
  const top = Math.max(outer.top, inner.top);
  const right = Math.min(outer.left + outer.width, inner.left + inner.width);
  const bottom = Math.min(outer.top + outer.height, inner.top + inner.height);
  const bands: Rect[] = [
    { left: outer.left, top: outer.top, width: outer.width, height: top - outer.top },
    { left: outer.left, top: bottom, width: outer.width, height: outer.top + outer.height - bottom },
    { left: outer.left, top, width: left - outer.left, height: bottom - top },
    { left: right, top, width: outer.left + outer.width - right, height: bottom - top },
  ];
  return bands.filter((band) => band.width > 0.001 && band.height > 0.001);
};

/** Shade over the parts of the page outside the kept area. */
const Shade = ({ rect }: { rect: Rect }) =>
  <>{bandsOutside({ left: 0, top: 0, width: 1, height: 1 }, rect).map((band, index) => <div key={index} className="crop-shade" style={boxStyle(band)} />)}</>;

type Drag =
  | { kind: "draw"; start: Rect; origin: { x: number; y: number } }
  | { kind: "move"; start: Rect; origin: { x: number; y: number } }
  | { kind: "resize"; start: Rect; handle: Handle };

/** The preview's file picker and pager, with room for actions on the right. */
const PreviewHeader = ({ files, selectedFileIndex, selectFile, pageIndex, pageCount, onPageChange, children }: {
  files: readonly string[];
  selectedFileIndex: number;
  selectFile: (index: number) => void;
  pageIndex: number;
  pageCount: number;
  onPageChange: (index: number) => void;
  children?: React.ReactNode;
}) => (
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
    {children}
  </div>
);

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
        <PreviewHeader files={files} selectedFileIndex={selectedFileIndex} selectFile={selectFile} pageIndex={pageIndex} pageCount={pageCount} onPageChange={onPageChange}>
          {history && (
            <>
              <Button variant="ghost" size="sm" disabled={!history.canUndo} onClick={history.onUndo} aria-keyshortcuts={isWindows() ? "Control+Z" : "Meta+Z"}>
                Undo<kbd aria-hidden="true" className="shortcut-hint">{undoKey}</kbd>
              </Button>
              <Button variant="ghost" size="sm" disabled={!history.canReset} onClick={history.onReset}>Reset crop</Button>
            </>
          )}
        </PreviewHeader>
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

/** The paper Resize sets pages to, how it is turned and where each page lands on it. */
export type ResizeTarget = { paper: PageSize; name: string; orientation: Orientation; placement: Placement };

/** "Letter", "landscape A4" or "58 × 120 mm": a page size for sentences. */
const pageNoun = (page: PageSize, unit: Unit) => {
  const name = paperName(page);
  if (!name) return showSize(page, unit);
  return name.endsWith(", landscape") ? `landscape ${name.slice(0, -", landscape".length)}` : name;
};
const withArticle = (noun: string) => `${/^([aeiou]|8|1[18](?!\d))/i.test(noun) ? "an" : "a"} ${noun}`;
const scaledTo = (scale: number) => `${scale > 1.01 ? "Scaled up to" : "Scaled to"} ${Math.round(scale * 100)}%`;

/** "16 mm is cut off the sides and 29 mm off the top and bottom.", totals across both edges of each axis. */
const cutSentence = ({ cut }: PagePlacement, unit: Unit) => {
  const length = (points: number) => `${showLength(points, unit)} ${unit}`;
  const sides = cut.width > PAPER_TOLERANCE;
  const ends = cut.height > PAPER_TOLERANCE;
  if (sides && ends) return `${length(cut.width)} is cut off the sides and ${length(cut.height)} off the top and bottom.`;
  return sides ? `${length(cut.width)} is cut off the sides.` : `${length(cut.height)} is cut off the top and bottom.`;
};

/** The new page and the scaled old page on one stage as large as both, in points from its top-left corner. */
const resizeLayout = (page: PageSize, { target, scale, offset }: PagePlacement) => {
  const content = { width: page.width * scale, height: page.height * scale };
  const width = Math.max(target.width, content.width);
  const height = Math.max(target.height, content.height);
  const paper = { left: (width - target.width) / 2, top: (height - target.height) / 2, width: target.width, height: target.height };
  const placed = { left: paper.left + offset.x, top: paper.top + offset.y, ...content };
  const fraction = (box: Rect): Rect => ({ left: box.left / width, top: box.top / height, width: box.width / width, height: box.height / height });
  return { width, height, paper: fraction(paper), content: fraction(placed) };
};

/** The extent a page takes on its stage: the page itself when it stays, or the new page and the placed page. */
const stageSize = (page: PageSize, placement: PagePlacement | null) => placement ? resizeLayout(page, placement) : page;

/** The new page with the old page placed on it; cut-off parts are hatched. A page that stays is drawn plain. */
const ResizeStage = ({ page, placement, pointsToPixels, image, alt, testId }: {
  page: PageSize;
  placement: PagePlacement | null;
  pointsToPixels: number;
  image?: string | null;
  alt: string;
  testId?: string;
}) => {
  const picture = image ? <img src={image} alt="" draggable={false} /> : null;
  if (!placement) {
    return (
      <div className="crop-stage" role="img" aria-label={alt} data-testid={testId} style={{ width: page.width * pointsToPixels, height: page.height * pointsToPixels }}>
        <div className="crop-page">{picture}</div>
      </div>
    );
  }
  const layout = resizeLayout(page, placement);
  return (
    <div
      className="crop-stage"
      role="img"
      aria-label={alt}
      data-testid={testId}
      data-target={`${placement.target.width.toFixed(2)} x ${placement.target.height.toFixed(2)}`}
      data-scale={placement.scale.toFixed(4)}
      data-offset={`${placement.offset.x.toFixed(2)} ${placement.offset.y.toFixed(2)}`}
      style={{ width: layout.width * pointsToPixels, height: layout.height * pointsToPixels }}
    >
      <div className="resize-paper" style={boxStyle(layout.paper)} />
      <div className="resize-content" style={boxStyle(layout.content)}>{picture}</div>
      {bandsOutside(layout.content, layout.paper).map((band, index) => <div key={index} className="resize-cut" data-testid="resize-cut" style={boxStyle(band)} />)}
      <div className="resize-target" style={boxStyle(layout.paper)} />
    </div>
  );
};

/**
 * Resize's preview: the previewed page on its new paper, scaled to fit or at actual size and centred, using the
 * same placement as the native resize, and the first page of each other size in the batch.
 */
export const ResizePreview = ({ files, selectedFileIndex, selectFile, sizes, pageIndex, onPageChange, target, unit }: {
  files: readonly string[];
  selectedFileIndex: number;
  selectFile: (index: number) => void;
  sizes: PageSizes;
  pageIndex: number;
  onPageChange: (index: number) => void;
  /** The paper and placement, or null while the size can't be read. */
  target: ResizeTarget | null;
  unit: Unit;
}) => {
  const path = files[selectedFileIndex];
  const pages = path === undefined ? undefined : sizes.get(path);
  const page = Array.isArray(pages) ? pages[pageIndex] : undefined;
  const image = usePagePreview(page ? path : undefined, pageIndex);
  const pageCount = Array.isArray(pages) ? pages.length : 0;
  const place = (size: PageSize) => target ? placePage(size, target.paper, target.orientation, target.placement) : null;
  const actual = target?.placement === "actualSize";

  const placement = page ? place(page) : null;
  const extent = page ? stageSize(page, placement) : null;
  const scale = extent ? Math.min(STAGE.width / extent.width, STAGE.height / extent.height) : 0;
  let caption: { text: string; tone?: "error" | "warning" } | null = null;
  let alt = `Page ${pageIndex + 1}`;
  if (page) {
    const noun = pageNoun(page, unit);
    if (!target) {
      caption = { text: "Fix the page size to see the result.", tone: "error" };
      alt = `Page ${pageIndex + 1}, ${noun}, unchanged.`;
    } else if (!placement) {
      caption = { text: `${describePage(page, unit)}. Already ${target.name}. Unchanged.` };
      alt = `Page ${pageIndex + 1}, ${noun}, unchanged.`;
    } else {
      const on = `${describePage(page, unit)} on ${describePage(placement.target, unit)}.`;
      const onPage = `${withArticle(pageNoun(placement.target, unit))} page`;
      if (!actual) {
        caption = { text: `${on} ${scaledTo(placement.scale)}.` };
        alt = `${noun} page scaled to fit ${onPage}, centered.`;
      } else if (cutsOff(placement)) {
        caption = { text: `${on} ${cutSentence(placement, unit)}`, tone: "warning" };
        alt = `${noun} page placed at actual size on ${onPage}. Hatched areas are cut off.`;
      } else {
        caption = { text: `${on} Fits. Nothing is cut off.` };
        alt = `${noun} page placed at actual size on ${onPage}, centered. Nothing is cut off.`;
      }
    }
  }

  const minis = page ? otherSizes(files, sizes, sizeKey(page)).map((mini) => ({ ...mini, placement: place(mini.page) })) : [];
  const miniScale = Math.min(1 / MINI.pointsPerPixel, ...minis.map((mini) => {
    const size = stageSize(mini.page, mini.placement);
    return Math.min(MINI.width / size.width, MINI.height / size.height);
  }));
  const hint = actual
    ? "Hatched areas are outside the new page and will be cut off."
    : target?.orientation === "portrait" ? "Every page gets portrait paper and is scaled to fit, so nothing is cut off."
      : target?.orientation === "landscape" ? "Every page gets landscape paper and is scaled to fit, so nothing is cut off."
        : "Each page keeps its orientation and is scaled to fit, so nothing is cut off.";

  return (
    <Card className="crop-preview py-0" role="region" aria-label="Preview">
      <CardContent className="p-0">
        <PreviewHeader files={files} selectedFileIndex={selectedFileIndex} selectFile={selectFile} pageIndex={pageIndex} pageCount={pageCount} onPageChange={onPageChange} />
        <div className="crop-preview-body">
          <figure className="crop-preview-figure">
            <div className="crop-preview-well">
              {pages === "error" ? (
                <span className="workspace-note">This PDF can't be previewed.</span>
              ) : !page ? (
                <span className="workspace-note">Loading the preview…</span>
              ) : (
                <ResizeStage page={page} placement={placement} pointsToPixels={scale} image={image} alt={alt} testId="resize-stage" />
              )}
            </div>
            {caption && (
              <figcaption
                className={`workspace-note${caption.tone === "error" ? " workspace-note-error" : caption.tone === "warning" ? " workspace-note-warning" : ""}`}
                data-testid="crop-preview-caption"
              >
                {caption.text}
              </figcaption>
            )}
          </figure>
          {minis.length > 0 && (
            <div className="crop-minis">
              <span className="crop-preview-label">On other page sizes</span>
              <div className="crop-minis-row">
                {minis.map((mini) => {
                  const name = paperName(mini.page) ?? showSize(mini.page, unit);
                  const cut = actual && cutsOff(mini.placement);
                  const result = !target ? null
                    : !mini.placement ? `Already ${target.name}. Unchanged.`
                      : actual ? cut ? "Content cut off" : "Fits. Nothing is cut off."
                        : `${target.orientation === "match" && mini.placement.target.width > mini.placement.target.height ? "Stays landscape. " : ""}${scaledTo(mini.placement.scale)}`;
                  const sub = !result ? name : mini.placement ? `${name}. ${result}` : result;
                  return (
                    <figure key={mini.title} className="crop-mini" data-testid="crop-mini">
                      <div className="crop-mini-stage">
                        <ResizeStage page={mini.page} placement={mini.placement} pointsToPixels={miniScale} alt={`${mini.title}: ${result ?? name}`} />
                      </div>
                      <figcaption>
                        <strong>{mini.title}</strong>
                        <span className={cut ? "workspace-note-warning" : undefined}>{sub}</span>
                      </figcaption>
                    </figure>
                  );
                })}
              </div>
              <p className="workspace-note">{hint}</p>
            </div>
          )}
        </div>
      </CardContent>
    </Card>
  );
};
