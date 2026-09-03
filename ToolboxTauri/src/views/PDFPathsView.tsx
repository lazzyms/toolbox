import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import type { ToolDefinition, ToolResult } from "../contracts";

export const PDFPathsView = ({ utility, mode }: { utility: ToolDefinition; mode: "merge" | "split" }) => {
    const [splitMode, setSplitMode] = useState<"pages" | "ranges" | "chunks">("pages");
    const [ranges, setRanges] = useState("");
    const [chunkSize, setChunkSize] = useState("2");
    return <ToolScaffold utility={utility} onRun={(paths) => mode === "merge"
        ? invoke<ToolResult>("merge_pdfs", { request: { paths, outputLocation: "alongsideInput" } })
        : invoke<ToolResult>("split_pdf", { request: { paths: [paths[0]], pages: [], pageRanges: splitMode === "ranges" ? ranges : null, splitMode, chunkSize: splitMode === "chunks" ? Number(chunkSize) : null, outputLocation: "alongsideInput" } })}>
        {({ files, run, loading }) => <div className="space-y-3">{mode === "merge" && <p className="text-sm text-slate-600" aria-live="polite">PDFs merge in the order they appear in the selected file list.</p>}{mode === "split" && <div className="flex flex-wrap gap-2"><label>Mode<select aria-label="Split mode" value={splitMode} onChange={(event) => setSplitMode(event.target.value as typeof splitMode)}><option value="pages">Every page</option><option value="ranges">Ranges</option><option value="chunks">Fixed-size chunks</option></select></label>{splitMode === "ranges" && <label>Ranges<input aria-label="Page ranges" placeholder="1-3, 4-8, 9-" value={ranges} onChange={(event) => setRanges(event.target.value)} /></label>}{splitMode === "chunks" && <label>Pages per file<input aria-label="Pages per file" type="number" min="1" value={chunkSize} onChange={(event) => setChunkSize(event.target.value)} /></label>}</div>}<button type="button" aria-label={mode === "merge" ? "Merge selected PDFs in order" : utility.shortTitle} disabled={loading || files.length === 0} onClick={run} className="rounded-lg bg-blue-600 px-6 py-2 font-medium text-white disabled:opacity-50">{utility.shortTitle}</button></div>}
    </ToolScaffold>;
};
