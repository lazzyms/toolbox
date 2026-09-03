import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import type { ToolDefinition, ToolResult } from "../contracts";

export const ImageMetadataView = ({ utility }: { utility: ToolDefinition }) => {
    const [report, setReport] = useState<string[]>([]);
    return <ToolScaffold utility={utility} onRun={(paths) => invoke<ToolResult>("image_metadata", { request: { paths, outputLocation: "alongsideInput" } })}>{({ files, run, loading }) => <div className="space-y-4"><div className="flex gap-3"><button type="button" disabled={loading || files.length === 0} onClick={async () => setReport(await invoke<string[]>("inspect_image_metadata", { request: { paths: files, outputLocation: "alongsideInput" } }))} className="rounded-lg border border-slate-300 px-4 py-2 font-medium disabled:opacity-50">Inspect</button><button type="button" disabled={loading || files.length === 0} onClick={run} className="rounded-lg bg-blue-600 px-6 py-2 font-medium text-white disabled:opacity-50">{utility.shortTitle}</button></div>{report.length > 0 && <pre className="text-xs text-slate-600">{JSON.stringify(report, null, 2)}</pre>}</div>}</ToolScaffold>;
};
