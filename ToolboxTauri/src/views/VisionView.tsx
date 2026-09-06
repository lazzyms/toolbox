import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import type { ToolDefinition, ToolResult } from "../contracts";

export const VisionView = ({ utility, mode }: { utility: ToolDefinition; mode: "ocr" | "faces" | "background" }) => (
    <ToolScaffold utility={utility} onRun={(paths) => invoke<ToolResult>(mode === "ocr" ? "ocr_pdf" : mode === "faces" ? "blur_faces" : "remove_image_background", { request: { paths, outputLocation: "alongsideInput" } })}>
        {({ files, run, loading }) => <div className="space-y-3"><p className="text-sm text-slate-600" aria-live="polite">{mode === "ocr" ? "Runs offline and writes normalized text beside the original PDF." : mode === "faces" ? "Runs through the configured offline face adapter." : "Runs through the configured offline background adapter."}</p><button type="button" aria-label={mode === "ocr" ? "Run offline OCR" : utility.shortTitle} disabled={loading || files.length === 0} onClick={run} className="rounded-lg bg-blue-600 px-6 py-2 font-medium text-white disabled:opacity-50">{utility.shortTitle}</button></div>}
    </ToolScaffold>
);
