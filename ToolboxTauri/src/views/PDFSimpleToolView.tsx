import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import type { ToolDefinition, ToolResult } from "../contracts";

type Mode = "pageNumbers" | "watermark" | "compress";

export const PDFSimpleToolView = ({ utility, mode }: { utility: ToolDefinition; mode: Mode }) => {
    const [text, setText] = useState("Toolbox");
    const [quality, setQuality] = useState(80);
    return (
        <ToolScaffold utility={utility} onRun={(paths) => {
            if (mode === "compress") return invoke<ToolResult>("compress_pdf", { request: { paths, quality, outputLocation: "alongsideInput" } });
            return invoke<ToolResult>(mode === "pageNumbers" ? "add_page_numbers" : "watermark_pdf", { request: { paths, text, outputLocation: "alongsideInput" } });
        }}>
            {({ files, run, loading }) => (
                <div className="space-y-4">
                    {mode === "compress" ? <input aria-label="PDF quality" type="range" min="1" max="100" value={quality} onChange={(event) => setQuality(Number(event.target.value))} /> : <input aria-label="Overlay text" value={text} onChange={(event) => setText(event.target.value)} className="rounded border px-3 py-2" />}
                    <button type="button" disabled={loading || files.length === 0} onClick={run} className="rounded-lg bg-blue-600 px-6 py-2 font-medium text-white disabled:opacity-50">{utility.shortTitle}</button>
                </div>
            )}
        </ToolScaffold>
    );
};
