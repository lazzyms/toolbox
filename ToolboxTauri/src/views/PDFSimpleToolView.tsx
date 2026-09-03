import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { ToolScaffold } from "../components/ToolScaffold";
import type { ToolDefinition, ToolResult } from "../contracts";

type Mode = "pageNumbers" | "watermark" | "compress";

export const PDFSimpleToolView = ({ utility, mode }: { utility: ToolDefinition; mode: Mode }) => {
    const [text, setText] = useState("Toolbox");
    const [quality, setQuality] = useState(80);
    const [opacity, setOpacity] = useState(70);
    const [position, setPosition] = useState("center");
    const [pages, setPages] = useState("");
    const [logoPath, setLogoPath] = useState<string | null>(null);
    const [startNumber, setStartNumber] = useState(1);
    const [fontSize, setFontSize] = useState(12);
    return (
        <ToolScaffold utility={utility} onRun={(paths) => {
            if (mode === "compress") return invoke<ToolResult>("compress_pdf", { request: { paths, quality, outputLocation: "alongsideInput" } });
            return invoke<ToolResult>(mode === "pageNumbers" ? "add_page_numbers" : "watermark_pdf", { request: { paths, text, opacity, position, logoPath, pages: pages ? pages.split(",").map((page) => Number(page.trim()) - 1) : null, startNumber, fontSize, outputLocation: "alongsideInput" } });
        }}>
            {({ files, run, loading }) => (
                <div className="space-y-4">
                    {mode === "compress" ? <input aria-label="PDF quality" type="range" min="1" max="100" value={quality} onChange={(event) => setQuality(Number(event.target.value))} /> : <div className="flex flex-wrap gap-2">{mode === "pageNumbers" ? <><label>Start<input aria-label="Starting page number" type="number" min="1" value={startNumber} onChange={(event) => setStartNumber(Number(event.target.value))} /></label><label>Font size<input aria-label="Page number font size" type="number" min="8" max="72" value={fontSize} onChange={(event) => setFontSize(Number(event.target.value))} /></label><label>Position<select aria-label="Page number position" value={position} onChange={(event) => setPosition(event.target.value)}><option value="bottom-right">Bottom right</option><option value="bottom-left">Bottom left</option><option value="top-right">Top right</option><option value="top-left">Top left</option></select></label><label>Pages<input aria-label="Page number pages" placeholder="all or 1, 3" value={pages} onChange={(event) => setPages(event.target.value)} /></label></> : <><input aria-label="Overlay text" value={text} onChange={(event) => setText(event.target.value)} className="rounded border px-3 py-2" /><label>Opacity<input aria-label="Watermark opacity" type="range" min="1" max="100" value={opacity} onChange={(event) => setOpacity(Number(event.target.value))} /></label><label>Position<select aria-label="Watermark position" value={position} onChange={(event) => setPosition(event.target.value)}><option value="center">Center</option><option value="top-left">Top left</option><option value="top-right">Top right</option><option value="bottom-left">Bottom left</option><option value="bottom-right">Bottom right</option></select></label><label>Pages<input aria-label="Watermark pages" placeholder="all or 1, 3" value={pages} onChange={(event) => setPages(event.target.value)} /></label><button type="button" onClick={async () => { const picked = await open({ multiple: false, filters: [{ name: "Watermark image", extensions: ["png", "jpg", "jpeg"] }] }); if (typeof picked === "string") setLogoPath(picked); }} className="rounded border px-2">{logoPath ? "Change logo" : "Add logo"}</button></>}</div>}
                    <button type="button" disabled={loading || files.length === 0} onClick={run} className="rounded-lg bg-blue-600 px-6 py-2 font-medium text-white disabled:opacity-50">{utility.shortTitle}</button>
                </div>
            )}
        </ToolScaffold>
    );
};
