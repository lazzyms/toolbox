import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../../components/ToolScaffold";
import { ShortcutHint } from "../../components/ShortcutHint";
import { shortcutTarget } from "../../shortcuts";
import type { ToolDefinition, ToolResult } from "../../contracts";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import type { PdfEditorBatchTool } from "./batchTools";

export const PdfEditorBatchSurface = ({ utility, tool, initialPaths, onFilesChange }: {
  utility: ToolDefinition;
  tool: PdfEditorBatchTool;
  initialPaths?: readonly string[];
  onFilesChange?: (paths: readonly string[]) => void;
}) => (
  <ToolScaffold
    variant="workspace"
    sessionKey="pdf-editor-batch"
    utility={utility}
    initialPaths={initialPaths}
    onFilesChange={onFilesChange}
    onRun={(paths, outputLocation, { runId }) => invoke<ToolResult>(utility.command, { runId, request: tool.request(paths, outputLocation) })}
  >
    {({ files, run, loading, destination, runLabel }) => (
      <Card className="workspace-control-panel py-0">
        <CardContent className="workspace-control-panel-content grid gap-4 p-4">
          <div>
            <h2 className="workspace-active-command">{utility.title}</h2>
            <p className="workspace-panel-copy">{tool.description}</p>
          </div>
          {destination}
          <Button
            variant="default"
            disabled={loading || files.length === 0 || utility.capability.nativeAvailability !== "available"}
            onClick={run}
            className="workspace-primary-action"
            aria-label={`Export ${utility.title}`}
            {...shortcutTarget("run")}
          >
            {runLabel ?? `Export ${utility.shortTitle}`}
            <ShortcutHint id="run" />
          </Button>
        </CardContent>
      </Card>
    )}
  </ToolScaffold>
);
