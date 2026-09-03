import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import type { ToolDefinition, ToolResult } from "../contracts";

export const VisionView = ({ utility, mode }: { utility: ToolDefinition; mode: "ocr" | "faces" | "background" }) => (
    <ToolScaffold utility={utility} onRun={(paths) => invoke<ToolResult>(mode === "ocr" ? "ocr_pdf" : mode === "faces" ? "blur_faces" : "remove_image_background", { request: { paths, outputLocation: "alongsideInput" } })}>
        {({ files, run, loading }) => <button type="button" disabled={loading || files.length === 0} onClick={run} className="rounded-lg bg-blue-600 px-6 py-2 font-medium text-white disabled:opacity-50">{utility.shortTitle}</button>}
    </ToolScaffold>
);
