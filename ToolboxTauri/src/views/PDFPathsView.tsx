import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import type { ToolDefinition, ToolResult } from "../contracts";

export const PDFPathsView = ({ utility, mode }: { utility: ToolDefinition; mode: "merge" | "split" }) => (
    <ToolScaffold utility={utility} onRun={(paths) => mode === "merge"
        ? invoke<ToolResult>("merge_pdfs", { request: { paths, outputLocation: "alongsideInput" } })
        : invoke<ToolResult>("split_pdf", { request: { paths: [paths[0]], pages: [], outputLocation: "alongsideInput" } })}>
        {({ files, run, loading }) => <button type="button" disabled={loading || files.length === 0} onClick={run} className="rounded-lg bg-blue-600 px-6 py-2 font-medium text-white disabled:opacity-50">{utility.shortTitle}</button>}
    </ToolScaffold>
);
