import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../../components/ToolScaffold";
import { ShortcutHint } from "../../components/ShortcutHint";
import { shortcutTarget } from "../../shortcuts";
import type { ToolDefinition, ToolResult } from "../../contracts";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { useToolAvailability } from "../../ToolAvailabilityContext";
import type { PdfEditorNavigation } from "../../views/PDFEditorWorkspaceView";
import type { PdfEditorBatchTool, ReportActions } from "./batchTools";

export const PdfEditorBatchSurface = ({ utility, tool, initialPaths, onFilesChange, onNavigate }: {
  utility: ToolDefinition;
  tool: PdfEditorBatchTool;
  initialPaths?: readonly string[];
  onFilesChange?: (paths: readonly string[]) => void;
  onNavigate?: PdfEditorNavigation;
}) => {
  const availability = useToolAvailability();
  const actions: ReportActions = {
    canOpen: (target) => Boolean(onNavigate) && availability.allows(target),
    open: (target, path) => onNavigate?.({ utilityId: target, initialPaths: [path] }),
  };
  const report = tool.report;
  return (
    <ToolScaffold
      variant="workspace"
      sessionKey="pdf-editor-batch"
      utility={utility}
      initialPaths={initialPaths}
      onFilesChange={onFilesChange}
      onRun={(paths, outputLocation, { runId }) => invoke<ToolResult>(utility.command, { runId, request: tool.request(paths, outputLocation) })}
      renderReport={report && ((results) => report.render(results, actions))}
    >
      {({ files, run, loading, destination, runLabel }) => (
        <Card className="workspace-control-panel py-0">
          <CardContent className="workspace-control-panel-content grid gap-4 p-4">
            <div>
              <h2 className="workspace-active-command">{utility.title}</h2>
              <p className="workspace-panel-copy">{tool.description}</p>
            </div>
            {tool.report ? <p className="workspace-note">{tool.report.note}</p> : destination}
            <Button
              variant="default"
              disabled={loading || files.length === 0 || utility.capability.nativeAvailability !== "available"}
              onClick={run}
              className="workspace-primary-action"
              aria-label={tool.report ? undefined : `Export ${utility.title}`}
              {...shortcutTarget("run")}
            >
              {tool.report?.runLabel(files.length) ?? runLabel ?? `Export ${utility.shortTitle}`}
              <ShortcutHint id="run" />
            </Button>
          </CardContent>
        </Card>
      )}
    </ToolScaffold>
  );
};
