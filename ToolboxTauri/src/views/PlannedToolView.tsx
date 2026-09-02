import type { ToolDefinition } from "../contracts";
import { ToolScaffold } from "../components/ToolScaffold";

export const PlannedToolView = ({ utility }: { utility: ToolDefinition }) => (
    <ToolScaffold utility={utility}>
        {() => (
            <div className="rounded-xl border border-amber-200 bg-amber-50 p-4 text-sm text-amber-800">
                This tool is listed in the migration matrix. Its Tauri command and verification are not implemented yet.
            </div>
        )}
    </ToolScaffold>
);
