import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import type { ToolDefinition, ToolResult } from "../contracts";

type Mode = "to-images" | "to-text" | "extract-images" | "images-to-pdf";

export const PDFConversionView = ({ utility, mode }: { utility: ToolDefinition; mode: Mode }) => (
    <ToolScaffold utility={utility} onRun={(paths) => {
        if (mode === "images-to-pdf") return invoke<ToolResult>("images_to_pdf", { request: { paths, outputLocation: "alongsideInput" } });
        if (mode === "to-images") return invoke<ToolResult>("pdf_to_images", { request: { paths, dpi: 150, format: "jpg", outputLocation: "alongsideInput" } });
        if (mode === "to-text") return invoke<ToolResult>("pdf_to_text", { request: { paths, outputLocation: "alongsideInput" } });
        return invoke<ToolResult>("extract_pdf_images", { request: { paths, outputLocation: "alongsideInput" } });
    }}>
        {({ files, run, loading }) => <div className="space-y-3">{mode === "to-text" && <p className="text-sm text-slate-600" aria-live="polite">Extracts selectable text in page order. Scanned PDFs are reported as requiring OCR.</p>}<button type="button" aria-label={mode === "to-text" ? "Extract selectable PDF text" : utility.shortTitle} disabled={loading || files.length === 0} onClick={run} className="rounded-lg bg-blue-600 px-6 py-2 font-medium text-white disabled:opacity-50">{utility.shortTitle}</button></div>}
    </ToolScaffold>
);
