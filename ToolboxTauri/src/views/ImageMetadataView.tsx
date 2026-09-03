import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import type { ToolDefinition, ToolResult } from "../contracts";

export const ImageMetadataView = ({ utility }: { utility: ToolDefinition }) => <ToolScaffold utility={utility} onRun={(paths) => invoke<ToolResult>("image_metadata", { request: { paths, outputLocation: "alongsideInput" } })}>{({ files, run, loading }) => <button type="button" disabled={loading || files.length === 0} onClick={run} className="rounded-lg bg-blue-600 px-6 py-2 font-medium text-white disabled:opacity-50">{utility.shortTitle}</button>}</ToolScaffold>;
