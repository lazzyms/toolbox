import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import type { ToolDefinition, ToolResult } from "../contracts";

export const ImageGeometryView = ({ utility, mode }: { utility: ToolDefinition; mode: "resize" | "rotate" | "crop" }) => {
    const [width, setWidth] = useState(1024); const [height, setHeight] = useState(768); const [degrees, setDegrees] = useState(90);
    const [cropMode, setCropMode] = useState("rectangle"); const [anchor, setAnchor] = useState("center");
    const [flip, setFlip] = useState("none");
    return <ToolScaffold utility={utility} onRun={(paths) => {
        if (mode === "resize") return invoke<ToolResult>("resize_images", { request: { paths, width, height, outputLocation: "alongsideInput" } });
        if (mode === "rotate") return invoke<ToolResult>("rotate_images", { request: { paths, degrees, flip, outputLocation: "alongsideInput" } });
        return invoke<ToolResult>("crop_images", { request: { paths, x: 0, y: 0, width, height, mode: cropMode, aspectWidth: width, aspectHeight: height, anchor, outputLocation: "alongsideInput" } });
    }}>
        {({ files, run, loading }) => <div className="space-y-4"><div className="flex gap-2">{mode === "crop" && <select aria-label="Crop mode" value={cropMode} onChange={(event) => setCropMode(event.target.value)} className="rounded border px-2 py-1"><option value="rectangle">Rectangle</option><option value="aspectRatio">Aspect ratio</option></select>}{mode !== "rotate" && <><input aria-label="Width" type="number" min="1" value={width} onChange={(event) => setWidth(Number(event.target.value))} className="w-24 rounded border px-2 py-1" /><input aria-label="Height" type="number" min="1" value={height} onChange={(event) => setHeight(Number(event.target.value))} className="w-24 rounded border px-2 py-1" /></>}</div>{mode === "crop" && cropMode === "aspectRatio" && <select aria-label="Crop anchor" value={anchor} onChange={(event) => setAnchor(event.target.value)} className="rounded border px-2 py-1"><option value="center">Center</option><option value="top">Top</option><option value="bottom">Bottom</option><option value="left">Left</option><option value="right">Right</option></select>}{mode === "rotate" && <><select aria-label="Rotation" value={degrees} onChange={(event) => setDegrees(Number(event.target.value))} className="rounded border px-2 py-1"><option value="0">0°</option><option value="90">90°</option><option value="180">180°</option><option value="270">270°</option></select><select aria-label="Mirror" value={flip} onChange={(event) => setFlip(event.target.value)} className="rounded border px-2 py-1"><option value="none">No mirror</option><option value="horizontal">Mirror horizontally</option><option value="vertical">Mirror vertically</option></select></>}{mode === "crop" && cropMode === "rectangle" && <span className="text-xs text-slate-500">Rectangle is validated against each image.</span>}<button type="button" disabled={loading || files.length === 0} onClick={run} className="rounded-lg bg-blue-600 px-6 py-2 font-medium text-white disabled:opacity-50">{utility.shortTitle}</button></div>}
    </ToolScaffold>;
};
