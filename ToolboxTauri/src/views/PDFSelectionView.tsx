import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import type { ToolDefinition, ToolResult } from "../contracts";
import type { PdfDocument } from "../features/pdf-editor";

export const PDFSelectionView = ({ utility, mode }: { utility: ToolDefinition; mode: "remove" | "extract" }) => {
    return mode === "remove" ? <PDFRemovalView utility={utility} /> : <PDFExtractionView utility={utility} />;
};

const PDFExtractionView = ({ utility }: { utility: ToolDefinition }) => {
    const [pages, setPages] = useState("1");
    return <ToolScaffold utility={utility} onRun={(paths) => {
        return invoke<ToolResult>("extract_pdf_pages", { request: { paths, pages: [], pageRanges: pages, outputLocation: "alongsideInput" } });
    }}>
        {({ files, run, loading }) => <div className="space-y-4"><label className="block text-sm">Pages or ranges<input aria-label="Page numbers or ranges" value={pages} onChange={(event) => setPages(event.target.value)} className="ml-3 rounded border px-3 py-2" placeholder="1-3, 7" /></label><button type="button" disabled={loading || files.length === 0} onClick={run} className="rounded-lg bg-blue-600 px-6 py-2 font-medium text-white disabled:opacity-50">{utility.shortTitle}</button></div>}
    </ToolScaffold>;
};

const PDFRemovalView = ({ utility }: { utility: ToolDefinition }) => {
    const [document, setDocument] = useState<PdfDocument | null>(null);
    const [selectedPages, setSelectedPages] = useState<number[]>([]);

    return <ToolScaffold utility={utility} onRun={(paths) => invoke<ToolResult>("remove_pdf_pages", { request: { paths, pages: selectedPages, pageRanges: null, outputLocation: "alongsideInput" } })}>
        {({ files, run, loading }) => <PDFRemovalContent files={files} run={run} loading={loading} document={document} setDocument={setDocument} selectedPages={selectedPages} setSelectedPages={setSelectedPages} utility={utility} />}
    </ToolScaffold>;
};

const PDFRemovalContent = ({ files, run, loading, document, setDocument, selectedPages, setSelectedPages, utility }: {
    files: string[]; run: () => Promise<void>; loading: boolean; document: PdfDocument | null;
    setDocument: (document: PdfDocument | null) => void; selectedPages: number[]; setSelectedPages: (pages: number[]) => void;
    utility: ToolDefinition;
}) => {
    useEffect(() => {
        const path = files[0];
        if (!path) { setDocument(null); setSelectedPages([]); return; }
        invoke<PdfDocument>("inspect_pdf", { request: { path } }).then((metadata) => {
            setDocument(metadata);
            setSelectedPages([]);
        }).catch(() => { setDocument(null); setSelectedPages([]); });
    }, [files, setDocument, setSelectedPages]);

    const togglePage = (page: number) => setSelectedPages(selectedPages.includes(page) ? selectedPages.filter((selected) => selected !== page) : [...selectedPages, page].sort((a, b) => a - b));
    return <div className="space-y-4">
        {document ? <fieldset><legend className="mb-2 text-sm font-medium">Select pages to remove</legend><div className="grid grid-cols-4 gap-2" role="group" aria-label="PDF pages">
            {document.pages.map((page) => <button key={page.index} type="button" aria-label={`Page ${page.index + 1}, ${page.width} by ${page.height} points`} aria-pressed={selectedPages.includes(page.index)} onClick={() => togglePage(page.index)} className={`rounded border px-3 py-4 text-sm ${selectedPages.includes(page.index) ? "border-red-600 bg-red-50 text-red-800" : "border-slate-300 bg-white"}`}>
                Page {page.index + 1}<span className="block text-xs text-slate-500">{page.width} × {page.height} pt</span>
            </button>)}
        </div></fieldset> : <p className="text-sm text-slate-500">Select a PDF to preview its pages.</p>}
        <p className="text-sm text-slate-600" aria-live="polite">{selectedPages.length} page{selectedPages.length === 1 ? "" : "s"} selected</p>
        <button type="button" disabled={loading || files.length === 0 || !document || selectedPages.length === 0} onClick={run} className="rounded-lg bg-blue-600 px-6 py-2 font-medium text-white disabled:opacity-50">{utility.shortTitle}</button>
    </div>;
};
