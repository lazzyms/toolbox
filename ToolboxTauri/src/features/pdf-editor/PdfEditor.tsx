import type { ReactNode } from "react";
import { useRef, useState } from "react";
import type { PdfDocument, PdfEditorState, PdfRect, PreviewSize } from "./contracts";
import { pdfToPreviewRect, previewToPdfRect, visiblePageIndices } from "./geometry";

interface PdfEditorProps {
    document: PdfDocument;
    state: PdfEditorState;
    onStateChange: (state: PdfEditorState) => void;
    onViewStateChange?: (state: PdfEditorState) => void;
    organizeControls?: boolean;
    pageSelectionMode?: "delete";
    renderOverlay?: (pageIndex: number) => ReactNode;
    selectionRectangle?: PdfRect | null;
    onSelectionChange?: (rectangle: PdfRect | null) => void;
    onUndo?: () => void;
    onRedo?: () => void;
    onReset?: () => void;
    canUndo?: boolean;
    canRedo?: boolean;
    canReset?: boolean;
}

const togglePageSelection = (state: PdfEditorState, pageIndex: number): PdfEditorState => {
    const selectedPages = state.selectedPages.includes(pageIndex)
        ? state.selectedPages.filter((index) => index !== pageIndex)
        : [...state.selectedPages, pageIndex].sort((a, b) => a - b);
    return {
        ...state,
        selectedPages,
        scope: { kind: "selected", pages: selectedPages },
    };
};

export const PdfEditor = ({ document, state, onStateChange, onViewStateChange, renderOverlay, organizeControls = false, pageSelectionMode, selectionRectangle, onSelectionChange, onUndo, onRedo, onReset, canUndo = false, canRedo = false, canReset = false }: PdfEditorProps) => {
    const updateViewState = onViewStateChange ?? onStateChange;
    const order = state.pageOrder.length ? state.pageOrder : document.pages.map((page) => page.index);
    const activeOrder = order.filter((index) => !state.deletedPages.includes(index));
    const currentPosition = Math.max(0, activeOrder.indexOf(state.currentPage));
    const visiblePages = visiblePageIndices(activeOrder.map((index) => document.pages[index]), currentPosition);
    const current = document.pages[state.currentPage];
    const [dragStart, setDragStart] = useState<{ x: number; y: number } | null>(null);
    const [zoom, setZoom] = useState(1);
    const [pan, setPan] = useState({ x: 0, y: 0 });
    const panStart = useRef<{ x: number; y: number; panX: number; panY: number } | null>(null);
    const [draggedPage, setDraggedPage] = useState<number | null>(null);
    const movePage = (delta: number) => { const next = Math.min(Math.max(0, currentPosition + delta), activeOrder.length - 1); updateViewState({ ...state, currentPage: activeOrder[next] }); };
    const reorderPage = (delta: number) => { const next = currentPosition + delta; if (next < 0 || next >= activeOrder.length) return; const nextOrder = [...order]; const from = nextOrder.indexOf(state.currentPage); const to = nextOrder.indexOf(activeOrder[next]); [nextOrder[from], nextOrder[to]] = [nextOrder[to], nextOrder[from]]; onStateChange({ ...state, pageOrder: nextOrder }); };
    const reorderDraggedPage = (pageIndex: number) => {
        if (draggedPage === null || draggedPage === pageIndex) return;
        const nextOrder = [...order];
        const from = nextOrder.indexOf(draggedPage);
        const to = nextOrder.indexOf(pageIndex);
        if (from < 0 || to < 0) return;
        nextOrder.splice(from, 1);
        nextOrder.splice(to, 0, draggedPage);
        onStateChange({ ...state, pageOrder: nextOrder, currentPage: draggedPage });
        setDraggedPage(null);
    };
    const rotatePage = () => { const existing = state.rotatePages.find((rotation) => rotation.page === state.currentPage); const degrees = ((existing?.degrees ?? 0) + 90) % 360; onStateChange({ ...state, rotatePages: [...state.rotatePages.filter((rotation) => rotation.page !== state.currentPage), { page: state.currentPage, degrees }] }); };
    const deletePage = () => { if (activeOrder.length <= 1) return; const deletedPages = [...new Set([...state.deletedPages, state.currentPage])]; const next = activeOrder[Math.min(currentPosition, activeOrder.length - 2)]; onStateChange({ ...state, deletedPages, currentPage: next }); };
    const previewSize: PreviewSize = current ? { width: 1000, height: 1000 * current.height / current.width } : { width: 1000, height: 1000 };
    const pointInPreview = (event: React.PointerEvent<HTMLDivElement>) => {
        const bounds = event.currentTarget.getBoundingClientRect();
        return {
            x: Math.min(Math.max((event.clientX - bounds.left) / bounds.width * previewSize.width, 0), previewSize.width),
            y: Math.min(Math.max((event.clientY - bounds.top) / bounds.height * previewSize.height, 0), previewSize.height),
        };
    };
    const beginSelection = (event: React.PointerEvent<HTMLDivElement>) => {
        if (!onSelectionChange || !current) return;
        setDragStart(pointInPreview(event));
        onSelectionChange(null);
        event.currentTarget.setPointerCapture(event.pointerId);
    };
    const updateSelection = (event: React.PointerEvent<HTMLDivElement>) => {
        if (!onSelectionChange || !current || !dragStart) return;
        const point = pointInPreview(event);
        const previewRectangle: PdfRect = {
            x: Math.min(dragStart.x, point.x),
            y: Math.min(dragStart.y, point.y),
            width: Math.abs(point.x - dragStart.x),
            height: Math.abs(point.y - dragStart.y),
        };
        if (previewRectangle.width > 0 && previewRectangle.height > 0) {
            onSelectionChange(previewToPdfRect(previewRectangle, current, previewSize));
        }
    };
    const finishSelection = (event: React.PointerEvent<HTMLDivElement>) => {
        updateSelection(event);
        setDragStart(null);
        if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    };
    const beginPan = (event: React.PointerEvent<HTMLDivElement>) => {
        if (onSelectionChange || zoom <= 1) return;
        panStart.current = { x: event.clientX, y: event.clientY, panX: pan.x, panY: pan.y };
        event.currentTarget.setPointerCapture(event.pointerId);
    };
    const updatePan = (event: React.PointerEvent<HTMLDivElement>) => {
        if (!panStart.current) return;
        setPan({ x: panStart.current.panX + event.clientX - panStart.current.x, y: panStart.current.panY + event.clientY - panStart.current.y });
    };
    const finishPan = (event: React.PointerEvent<HTMLDivElement>) => {
        panStart.current = null;
        if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    };
    const cropPreview = current && selectionRectangle ? pdfToPreviewRect(selectionRectangle, current, previewSize) : null;

    return (
        <div className="grid min-h-0 grid-cols-[minmax(96px,160px)_minmax(0,1fr)_minmax(160px,220px)] gap-4" role="region" aria-label="PDF editor" tabIndex={0} onKeyDown={(event) => {
            if (event.key === "ArrowLeft" || event.key === "ArrowUp") { event.preventDefault(); movePage(-1); }
            if (event.key === "ArrowRight" || event.key === "ArrowDown") { event.preventDefault(); movePage(1); }
        }}>
            <aside aria-label="PDF page thumbnails" className={`flex min-h-0 flex-col gap-2 overflow-auto rounded-xl border border-slate-200 bg-white p-2 ${state.density === "compact" ? "text-xs" : ""}`}>
                {visiblePages.map((pageIndex) => {
                    const page = document.pages[pageIndex];
                    const selected = pageSelectionMode === "delete" ? state.deletedPages.includes(pageIndex) : state.selectedPages.includes(pageIndex);
                    return (
                        <button
                            key={page.index}
                            type="button"
                            aria-label={`Page ${page.index + 1}, ${page.width} by ${page.height} points`}
                            aria-pressed={selected}
                            draggable={true}
                            onDragStart={() => setDraggedPage(pageIndex)}
                            onDragOver={(event) => event.preventDefault()}
                            onDrop={() => reorderDraggedPage(pageIndex)}
                            onClick={(event) => {
                                if (pageSelectionMode === "delete") {
                                    const from = activeOrder.indexOf(state.currentPage);
                                    const to = activeOrder.indexOf(pageIndex);
                                    const [start, end] = [Math.min(from, to), Math.max(from, to)];
                                    const targets = event.shiftKey || event.metaKey || event.ctrlKey ? activeOrder.slice(start, end + 1) : [pageIndex];
                                    const allSelected = targets.every((target) => state.deletedPages.includes(target));
                                    const nextDeletedPages = allSelected
                                        ? state.deletedPages.filter((index) => !targets.includes(index))
                                        : [...new Set([...state.deletedPages, ...targets])].sort((a, b) => a - b);
                                    const nextCurrentPage = activeOrder.find((index) => !nextDeletedPages.includes(index)) ?? pageIndex;
                                    onStateChange({
                                        ...state,
                                        currentPage: nextCurrentPage,
                                        deletedPages: nextDeletedPages,
                                    });
                                } else if (event.shiftKey || event.metaKey || event.ctrlKey) {
                                    const from = activeOrder.indexOf(state.currentPage);
                                    const to = activeOrder.indexOf(pageIndex);
                                    const [start, end] = [Math.min(from, to), Math.max(from, to)];
                                    const range = activeOrder.slice(start, end + 1);
                                    updateViewState({ ...state, currentPage: pageIndex, selectedPages: [...new Set([...state.selectedPages, ...range])].sort((a, b) => a - b), scope: { kind: "selected", pages: [...new Set([...state.selectedPages, ...range])].sort((a, b) => a - b) } });
                                } else {
                                    updateViewState({ ...state, currentPage: pageIndex });
                                }
                            }}
                            className={`relative shrink-0 rounded border p-2 ${selected ? "border-blue-500 ring-2 ring-blue-200" : "border-slate-200"}`}
                        >
                            {page.preview ? <img
                                src={page.preview}
                                alt={`Thumbnail of page ${page.index + 1}`}
                                className="block bg-slate-100 object-contain"
                                draggable={false}
                                style={{ width: state.density === "compact" ? 48 : 64, height: (state.density === "compact" ? 48 : 64) * page.height / page.width }}
                            /> : <span className="block bg-slate-100" style={{ width: state.density === "compact" ? 48 : 64, height: (state.density === "compact" ? 48 : 64) * page.height / page.width }} />}
                            <span className="text-xs text-slate-500">{page.index + 1}</span>
                        </button>
                    );
                })}
            </aside>

            {current ? (
                <main aria-label={`Page canvas ${current.index + 1}`} className="min-w-0">
                    <div className="mb-2 flex flex-wrap items-center gap-2 text-sm">
                        <button type="button" aria-label="Zoom out" onClick={() => setZoom((value) => Math.max(0.5, Number((value - 0.25).toFixed(2))))} disabled={zoom <= 0.5}>−</button>
                        <span aria-label="Current zoom">{Math.round(zoom * 100)}%</span>
                        <button type="button" aria-label="Zoom in" onClick={() => setZoom((value) => Math.min(3, Number((value + 0.25).toFixed(2))))} disabled={zoom >= 3}>+</button>
                        <button type="button" aria-label="Reset view" onClick={() => { setZoom(1); setPan({ x: 0, y: 0 }); }}>Fit</button>
                        {onUndo && <button type="button" aria-label="Undo" title="Undo" onClick={onUndo} disabled={!canUndo}>↶</button>}
                        {onRedo && <button type="button" aria-label="Redo" title="Redo" onClick={onRedo} disabled={!canRedo}>↷</button>}
                        {onReset && <button type="button" aria-label="Reset edits" onClick={onReset} disabled={!canReset}>Reset edits</button>}
                        <button type="button" aria-label="Previous" title="Previous page" onClick={() => movePage(-1)} disabled={currentPosition === 0}>‹</button>
                        <span>Page {current.index + 1} of {document.pages.length}</span>
                        <button type="button" aria-label="Next" title="Next page" onClick={() => movePage(1)} disabled={currentPosition === activeOrder.length - 1}>›</button>
                        <button type="button" onClick={() => updateViewState({ ...state, scope: { kind: "all" } })}>All pages</button>
                        <button type="button" onClick={() => updateViewState(togglePageSelection(state, current.index))}>Select page</button>
                        {organizeControls && <><button type="button" onClick={() => reorderPage(-1)} disabled={currentPosition === 0}>Move left</button><button type="button" onClick={() => reorderPage(1)} disabled={currentPosition === activeOrder.length - 1}>Move right</button><button type="button" onClick={rotatePage}>Rotate 90°</button><button type="button" onClick={deletePage} disabled={activeOrder.length <= 1}>Delete page</button></>}
                        <button type="button" onClick={() => updateViewState({ ...state, layout: state.layout === "vertical" ? "horizontal" : "vertical" })}>Change layout</button>
                    </div>
                    <div className="relative mx-auto max-w-2xl overflow-auto border border-slate-300 bg-white shadow-sm" style={{ aspectRatio: `${current.width} / ${current.height}`, maxHeight: "70vh", cursor: zoom > 1 && !onSelectionChange ? "grab" : undefined }}>
                        <div
                            className={`relative min-h-full min-w-full bg-slate-50 ${onSelectionChange ? "cursor-crosshair" : ""}`}
                            aria-label={`Preview of page ${current.index + 1}`}
                            style={{ aspectRatio: `${current.width} / ${current.height}`, transform: `translate(${pan.x}px, ${pan.y}px) scale(${zoom})`, transformOrigin: "center center", touchAction: "none" }}
                            onPointerDown={beginSelection}
                            onPointerMove={updateSelection}
                            onPointerUp={finishSelection}
                            onPointerDownCapture={beginPan}
                            onPointerMoveCapture={updatePan}
                            onPointerUpCapture={finishPan}
                        >
                            {current.preview ? <img src={current.preview} alt={`Preview of page ${current.index + 1}`} className="absolute inset-0 h-full w-full object-contain" draggable={false} /> : <span className="absolute inset-0 flex items-center justify-center text-sm text-slate-500">Preview unavailable.</span>}
                            {cropPreview && <div
                                aria-label="Crop rectangle"
                                className="pointer-events-none absolute border-2 border-blue-600 bg-blue-200/30"
                                style={{
                                    left: `${cropPreview.x / previewSize.width * 100}%`,
                                    top: `${cropPreview.y / previewSize.height * 100}%`,
                                    width: `${cropPreview.width / previewSize.width * 100}%`,
                                    height: `${cropPreview.height / previewSize.height * 100}%`,
                                }}
                            />}
                            {onSelectionChange && !selectionRectangle && <span className="pointer-events-none absolute inset-x-0 top-1/2 text-center text-sm text-slate-500">Drag over the page to select a crop rectangle.</span>}
                            {renderOverlay && <div role="region" aria-label="Live PDF overlay" className="pointer-events-none absolute inset-0">{renderOverlay(current.index)}</div>}
                        </div>
                    </div>
                </main>
            ) : <p role="status">No PDF pages are available.</p>}
            <aside aria-label="PDF editor inspector" className="rounded-xl border border-slate-200 bg-white p-3 text-sm">
                <h3 className="font-semibold text-slate-700">Inspector</h3>
                <p className="mt-2 text-slate-500">{state.selectedPages.length} pages selected</p>
                <p className="text-slate-500">Scope: {state.scope.kind === "all" ? "all pages" : "selected pages"}</p>
                <div className="mt-3 flex flex-wrap gap-2">
                    <button type="button" onClick={() => updateViewState({ ...state, selectedPages: activeOrder, scope: { kind: "selected", pages: activeOrder } })}>Select all</button>
                    <button type="button" onClick={() => updateViewState({ ...state, selectedPages: [], scope: { kind: "all" } })}>Clear selection</button>
                </div>
                <button type="button" className="mt-4" onClick={() => updateViewState({ ...state, density: state.density === "compact" ? "comfortable" : "compact" })}>
                    Density: {state.density}
                </button>
                <p className="mt-4 min-h-5 text-red-600" role="alert" aria-live="assertive">{state.error ?? ""}</p>
            </aside>
        </div>
    );
};
