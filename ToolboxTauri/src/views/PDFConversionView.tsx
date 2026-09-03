import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import type { ToolDefinition, ToolResult } from "../contracts";

type Mode = "to-images" | "to-text" | "extract-images" | "images-to-pdf";

export const PDFConversionView = ({ utility, mode }: { utility: ToolDefinition; mode: Mode }) => {
    const [dpi, setDpi] = useState("150");
    const [format, setFormat] = useState("jpg");
    const [pageRange, setPageRange] = useState("");
    return <ToolScaffold utility={utility} onRun={(paths) => {
        if (mode === "images-to-pdf") return invoke<ToolResult>("images_to_pdf", { request: { paths, outputLocation: "alongsideInput" } });
        if (mode === "to-images") return invoke<ToolResult>("pdf_to_images", { request: { paths, dpi: Number(dpi), format, pageRange: pageRange || null, outputLocation: "alongsideInput" } });
        if (mode === "to-text") return invoke<ToolResult>("pdf_to_text", { request: { paths, outputLocation: "alongsideInput" } });
        return invoke<ToolResult>("extract_pdf_images", { request: { paths, outputLocation: "alongsideInput" } });
    }}>
        {({ files, run, loading }) => <div className="space-y-3">{mode === "to-images" && <div className="flex flex-wrap gap-2"><label>DPI<select aria-label="Render DPI" value={dpi} onChange={(event) => setDpi(event.target.value)}><option>72</option><option>150</option><option>300</option></select></label><label>Format<select aria-label="Image format" value={format} onChange={(event) => setFormat(event.target.value)}><option value="jpg">JPEG</option><option value="png">PNG</option></select></label><label>Pages<input aria-label="Page range" placeholder="all or 1-3" value={pageRange} onChange={(event) => setPageRange(event.target.value)} /></label></div>}{mode === "to-text" && <p className="text-sm text-slate-600" aria-live="polite">Extracts selectable text in page order. Scanned PDFs are reported as requiring OCR.</p>}<button type="button" aria-label={mode === "to-text" ? "Extract selectable PDF text" : utility.shortTitle} disabled={loading || files.length === 0} onClick={run} className="rounded-lg bg-blue-600 px-6 py-2 font-medium text-white disabled:opacity-50">{utility.shortTitle}</button></div>}
    </ToolScaffold>;
};
