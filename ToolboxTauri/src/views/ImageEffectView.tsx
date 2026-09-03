import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import type { ToolDefinition, ToolResult } from "../contracts";

export const ImageEffectView = ({ utility, mode }: { utility: ToolDefinition; mode: "tone" | "watermark" }) => {
    const [value, setValue] = useState(mode === "tone" ? 0 : 20);
    return <ToolScaffold utility={utility} onRun={(paths) => mode === "tone" ? invoke<ToolResult>("adjust_image_tone", { request: { paths, brightness: value, contrast: 0, outputLocation: "alongsideInput" } }) : invoke<ToolResult>("watermark_images", { request: { paths, opacity: value, outputLocation: "alongsideInput" } })}>
        {({ files, run, loading }) => <div className="space-y-4"><label className="block text-sm">{mode === "tone" ? "Brightness" : "Watermark opacity"}<input aria-label={mode === "tone" ? "Brightness" : "Watermark opacity"} type="range" min={mode === "tone" ? -100 : 0} max="100" value={value} onChange={(event) => setValue(Number(event.target.value))} /></label><button type="button" disabled={loading || files.length === 0} onClick={run} className="rounded-lg bg-blue-600 px-6 py-2 font-medium text-white disabled:opacity-50">{utility.shortTitle}</button></div>}
    </ToolScaffold>;
};
