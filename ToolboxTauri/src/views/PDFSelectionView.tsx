import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import type { ToolDefinition, ToolResult } from "../contracts";

export const PDFSelectionView = ({ utility, mode }: { utility: ToolDefinition; mode: "remove" | "extract" }) => {
    const [pages, setPages] = useState("1");
    return <ToolScaffold utility={utility} onRun={(paths) => {
        return invoke<ToolResult>(mode === "remove" ? "remove_pdf_pages" : "extract_pdf_pages", { request: { paths, pages: [], pageRanges: pages, outputLocation: "alongsideInput" } });
    }}>
        {({ files, run, loading }) => <div className="space-y-4"><label className="block text-sm">Pages or ranges<input aria-label="Page numbers or ranges" value={pages} onChange={(event) => setPages(event.target.value)} className="ml-3 rounded border px-3 py-2" placeholder="1-3, 7" /></label><button type="button" disabled={loading || files.length === 0} onClick={run} className="rounded-lg bg-blue-600 px-6 py-2 font-medium text-white disabled:opacity-50">{utility.shortTitle}</button></div>}
    </ToolScaffold>;
};
