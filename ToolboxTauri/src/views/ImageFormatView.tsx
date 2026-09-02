import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import type { ToolDefinition, ToolResult } from "../contracts";

export const ImageFormatView = ({ utility, mode }: { utility: ToolDefinition; mode: "icons" | "gif-create" | "gif-extract" | "tiff" }) => (
    <ToolScaffold utility={utility} onRun={(paths) => {
        if (mode === "icons") return invoke<ToolResult>("generate_icon_set", { request: { paths: [paths[0]], sizes: [16, 32, 64, 128, 256, 512, 1024], outputLocation: "alongsideInput" } });
        if (mode === "gif-create") return invoke<ToolResult>("create_gif", { request: { paths, outputLocation: "alongsideInput" } });
        if (mode === "gif-extract") return invoke<ToolResult>("extract_gif_frames", { request: { paths, outputLocation: "alongsideInput" } });
        return invoke<ToolResult>("process_tiff_pages", { request: { paths, outputLocation: "alongsideInput" } });
    }}>
        {({ files, run, loading }) => <button type="button" disabled={loading || files.length === 0} onClick={run} className="rounded-lg bg-blue-600 px-6 py-2 font-medium text-white disabled:opacity-50">{utility.shortTitle}</button>}
    </ToolScaffold>
);
