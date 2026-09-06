import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import type { ToolDefinition, ToolResult } from "../contracts";

export const ImageFormatView = ({ utility, mode }: { utility: ToolDefinition; mode: "icons" | "gif-create" | "gif-extract" | "tiff" }) => {
    const [delay, setDelay] = useState(100);
    const [loop, setLoop] = useState(true);
    const [iconPreset, setIconPreset] = useState("macos");
    return <ToolScaffold utility={utility} onRun={(paths) => {
        if (mode === "icons") return invoke<ToolResult>("generate_icon_set", { request: { paths: [paths[0]], preset: iconPreset, sizes: [], outputLocation: "alongsideInput" } });
        if (mode === "gif-create") return invoke<ToolResult>("create_gif", { request: { paths, frameDelayMs: delay, loopForever: loop, outputLocation: "alongsideInput" } });
        if (mode === "gif-extract") return invoke<ToolResult>("extract_gif_frames", { request: { paths, outputLocation: "alongsideInput" } });
        return invoke<ToolResult>("process_tiff_pages", { request: { paths, outputLocation: "alongsideInput" } });
    }}>
        {({ files, run, loading }) => <div className="space-y-4">{mode === "icons" && <label className="block text-sm">Icon preset<select aria-label="Icon preset" value={iconPreset} onChange={(event) => setIconPreset(event.target.value)} className="ml-3 rounded border px-2 py-1"><option value="macos">macOS</option><option value="favicon">Favicon</option><option value="ios">iOS</option><option value="android">Android</option></select></label>}{mode === "gif-create" && <><label className="block text-sm">Frame delay (ms)<input aria-label="Frame delay in milliseconds" type="number" min="1" max="60000" value={delay} onChange={(event) => setDelay(Number(event.target.value))} className="ml-3 w-24 rounded border px-2 py-1" /></label><label className="flex items-center gap-2 text-sm"><input aria-label="Loop animation" type="checkbox" checked={loop} onChange={(event) => setLoop(event.target.checked)} />Loop animation</label></>}<button type="button" disabled={loading || files.length === 0} onClick={run} className="rounded-lg bg-blue-600 px-6 py-2 font-medium text-white disabled:opacity-50">{utility.shortTitle}</button></div>}
    </ToolScaffold>
};
