import type { ReactNode } from "react";
import { useState } from "react";
import type { PdfDocument, PdfEditorState, PdfRect, PreviewSize } from "./contracts";
import { pdfToPreviewRect, previewToPdfRect, visiblePageIndices } from "./geometry";

interface PdfEditorProps {
    document: PdfDocument;
    state: PdfEditorState;
    onStateChange: (state: PdfEditorState) => void;
    organizeControls?: boolean;
    renderOverlay?: (pageIndex: number) => ReactNode;
    selectionRectangle?: PdfRect | null;
    onSelectionChange?: (rectangle: PdfRect | null) => void;
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

export const PdfEditor = ({ document, state, onStateChange, renderOverlay, organizeControls = false, selectionRectangle, onSelectionChange }: PdfEditorProps) => {
    const order = state.pageOrder.length ? state.pageOrder : document.pages.map((page) => page.index);
    const activeOrder = order.filter((index) => !state.deletedPages.includes(index));
    const currentPosition = Math.max(0, activeOrder.indexOf(state.currentPage));
    const visiblePages = visiblePageIndices(activeOrder.map((index) => document.pages[index]), currentPosition);
    const current = document.pages[state.currentPage];
    const [dragStart, setDragStart] = useState<{ x: number; y: number } | null>(null);
    const movePage = (delta: number) => { const next = Math.min(Math.max(0, currentPosition + delta), activeOrder.length - 1); onStateChange({ ...state, currentPage: activeOrder[next] }); };
    const reorderPage = (delta: number) => { const next = currentPosition + delta; if (next < 0 || next >= activeOrder.length) return; const nextOrder = [...order]; const from = nextOrder.indexOf(state.currentPage); const to = nextOrder.indexOf(activeOrder[next]); [nextOrder[from], nextOrder[to]] = [nextOrder[to], nextOrder[from]]; onStateChange({ ...state, pageOrder: nextOrder }); };
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
    const cropPreview = current && selectionRectangle ? pdfToPreviewRect(selectionRectangle, current, previewSize) : null;

    return (
        <div className="grid min-h-0 grid-cols-[minmax(96px,160px)_minmax(0,1fr)_minmax(160px,220px)] gap-4" role="region" aria-label="PDF editor" tabIndex={0} onKeyDown={(event) => {
            if (event.key === "ArrowLeft" || event.key === "ArrowUp") { event.preventDefault(); movePage(-1); }
            if (event.key === "ArrowRight" || event.key === "ArrowDown") { event.preventDefault(); movePage(1); }
        }}>
            <aside aria-label="PDF page thumbnails" className={`flex min-h-0 flex-col gap-2 overflow-auto rounded-xl border border-slate-200 bg-white p-2 ${state.density === "compact" ? "text-xs" : ""}`}>
                {visiblePages.map((pageIndex) => {
                    const page = document.pages[pageIndex];
                    const selected = state.selectedPages.includes(pageIndex);
                    return (
                        <button
                            key={page.index}
                            type="button"
                            aria-label={`Page ${page.index + 1}`}
                            aria-pressed={selected}
                            onClick={() => onStateChange({ ...state, currentPage: pageIndex })}
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
                        <button type="button" onClick={() => movePage(-1)} disabled={state.currentPage === 0}>Previous</button>
                        <span>Page {current.index + 1} of {document.pages.length}</span>
                        <button type="button" onClick={() => movePage(1)} disabled={state.currentPage === document.pages.length - 1}>Next</button>
                        <button type="button" onClick={() => onStateChange({ ...state, scope: { kind: "all" } })}>All pages</button>
                        <button type="button" onClick={() => onStateChange(togglePageSelection(state, current.index))}>Select page</button>
                        {organizeControls && <><button type="button" onClick={() => reorderPage(-1)} disabled={currentPosition === 0}>Move left</button><button type="button" onClick={() => reorderPage(1)} disabled={currentPosition === activeOrder.length - 1}>Move right</button><button type="button" onClick={rotatePage}>Rotate 90°</button><button type="button" onClick={deletePage} disabled={activeOrder.length <= 1}>Delete page</button></>}
                        <button type="button" onClick={() => onStateChange({ ...state, layout: state.layout === "vertical" ? "horizontal" : "vertical" })}>Change layout</button>
                    </div>
                    <div className="relative mx-auto max-w-2xl overflow-hidden border border-slate-300 bg-white shadow-sm" style={{ aspectRatio: `${current.width} / ${current.height}` }}>
                        <div
                            className={`absolute inset-0 bg-slate-50 ${onSelectionChange ? "cursor-crosshair" : ""}`}
                            aria-label={`Preview of page ${current.index + 1}`}
                            style={{ touchAction: onSelectionChange ? "none" : undefined }}
                            onPointerDown={beginSelection}
                            onPointerMove={updateSelection}
                            onPointerUp={finishSelection}
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
                        </div>
                        {renderOverlay?.(current.index)}
                    </div>
                </main>
            ) : <p role="status">No PDF pages are available.</p>}
            <aside aria-label="PDF editor inspector" className="rounded-xl border border-slate-200 bg-white p-3 text-sm">
                <h3 className="font-semibold text-slate-700">Inspector</h3>
                <p className="mt-2 text-slate-500">{state.selectedPages.length} pages selected</p>
                <p className="text-slate-500">Scope: {state.scope.kind === "all" ? "all pages" : "selected pages"}</p>
                <button type="button" className="mt-4" onClick={() => onStateChange({ ...state, density: state.density === "compact" ? "comfortable" : "compact" })}>
                    Density: {state.density}
                </button>
                <p className="mt-4 min-h-5 text-red-600" role="alert" aria-live="assertive">{state.error ?? ""}</p>
            </aside>
        </div>
    );
};
