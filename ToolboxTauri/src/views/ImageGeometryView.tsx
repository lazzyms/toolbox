import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import type { ToolDefinition, ToolResult } from "../contracts";

export const ImageGeometryView = ({ utility, mode }: { utility: ToolDefinition; mode: "resize" | "rotate" | "crop" }) => {
    const [width, setWidth] = useState(1024); const [height, setHeight] = useState(768); const [degrees, setDegrees] = useState(90);
    return <ToolScaffold utility={utility} onRun={(paths) => {
        if (mode === "resize") return invoke<ToolResult>("resize_images", { request: { paths, width, height, outputLocation: "alongsideInput" } });
        if (mode === "rotate") return invoke<ToolResult>("rotate_images", { request: { paths, degrees, outputLocation: "alongsideInput" } });
        return invoke<ToolResult>("crop_images", { request: { paths, x: 0, y: 0, width, height, outputLocation: "alongsideInput" } });
    }}>
        {({ files, run, loading }) => <div className="space-y-4"><div className="flex gap-2"><input aria-label="Width" type="number" value={width} onChange={(event) => setWidth(Number(event.target.value))} className="w-24 rounded border px-2 py-1" /><input aria-label="Height" type="number" value={height} onChange={(event) => setHeight(Number(event.target.value))} className="w-24 rounded border px-2 py-1" /></div>{mode === "rotate" && <select aria-label="Rotation" value={degrees} onChange={(event) => setDegrees(Number(event.target.value))} className="rounded border px-2 py-1"><option value="90">90°</option><option value="180">180°</option><option value="270">270°</option></select>}<button type="button" disabled={loading || files.length === 0} onClick={run} className="rounded-lg bg-blue-600 px-6 py-2 font-medium text-white disabled:opacity-50">{utility.shortTitle}</button></div>}
    </ToolScaffold>;
};
