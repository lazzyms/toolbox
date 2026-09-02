import type { ReactNode } from "react";
import type { PdfDocument, PdfEditorState } from "./contracts";
import { visiblePageIndices } from "./geometry";

interface PdfEditorProps {
    document: PdfDocument;
    state: PdfEditorState;
    onStateChange: (state: PdfEditorState) => void;
    renderOverlay?: (pageIndex: number) => ReactNode;
}

const updateSelection = (state: PdfEditorState, pageIndex: number): PdfEditorState => {
    const selectedPages = state.selectedPages.includes(pageIndex)
        ? state.selectedPages.filter((index) => index !== pageIndex)
        : [...state.selectedPages, pageIndex].sort((a, b) => a - b);
    return {
        ...state,
        selectedPages,
        scope: { kind: "selected", pages: selectedPages },
    };
};

export const PdfEditor = ({ document, state, onStateChange, renderOverlay }: PdfEditorProps) => {
    const visiblePages = visiblePageIndices(document.pages, state.currentPage);
    const current = document.pages[state.currentPage];

    return (
        <div className={`flex gap-4 ${state.layout === "horizontal" ? "flex-row" : "flex-col"}`}>
            <aside className="flex gap-2 overflow-auto rounded-xl border border-slate-200 bg-white p-2">
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
                            <span className="block bg-slate-100" style={{ width: 64, height: 64 * page.height / page.width }} />
                            <span className="text-xs text-slate-500">{page.index + 1}</span>
                        </button>
                    );
                })}
            </aside>

            {current && (
                <div className="min-w-0 flex-1">
                    <div className="mb-2 flex flex-wrap items-center gap-2 text-sm">
                        <button type="button" onClick={() => onStateChange({ ...state, currentPage: Math.max(0, state.currentPage - 1) })} disabled={state.currentPage === 0}>Previous</button>
                        <span>Page {current.index + 1} of {document.pages.length}</span>
                        <button type="button" onClick={() => onStateChange({ ...state, currentPage: Math.min(document.pages.length - 1, state.currentPage + 1) })} disabled={state.currentPage === document.pages.length - 1}>Next</button>
                        <button type="button" onClick={() => onStateChange({ ...state, scope: { kind: "all" } })}>All pages</button>
                        <button type="button" onClick={() => onStateChange(updateSelection(state, current.index))}>Select page</button>
                        <button type="button" onClick={() => onStateChange({ ...state, layout: state.layout === "vertical" ? "horizontal" : "vertical" })}>Change layout</button>
                    </div>
                    <div className="relative mx-auto max-w-2xl overflow-hidden border border-slate-300 bg-white shadow-sm" style={{ aspectRatio: `${current.width} / ${current.height}` }}>
                        <div className="absolute inset-0 bg-slate-50" aria-label={`Preview of page ${current.index + 1}`} />
                        {renderOverlay?.(current.index)}
                    </div>
                </div>
            )}
        </div>
    );
};
