import type { ToolDefinition } from "../contracts";
import { ToolScaffold } from "../components/ToolScaffold";

export const PlannedToolView = ({ utility }: { utility: ToolDefinition }) => (
    <ToolScaffold utility={utility} onRun={async () => []}>
        {() => (
            <div className="rounded-xl border border-amber-200 bg-amber-50 p-4 text-sm text-amber-800">
                This tool is listed in the migration matrix. Its Tauri command and verification are not implemented yet.
            </div>
        )}
    </ToolScaffold>
);

export const UnavailableToolView = ({ utility }: { utility: ToolDefinition }) => (
    <div className="flex h-full flex-col">
        <div className="mb-6">
            <h2 className="text-3xl font-bold text-slate-900">{utility.title}</h2>
            <p className="text-slate-500">{utility.blurb}</p>
        </div>
        <div className="rounded-xl border border-slate-200 bg-white p-5 text-sm text-slate-700" role="status">
            <p className="font-semibold">Unavailable in this build.</p>
            <p className="mt-2">This tool requires an offline vision resource that is not bundled. Files will not be selected or processed.</p>
        </div>
    </div>
);
