import { useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../../components/ToolScaffold";
import { BatchPasswordFields, FilePasswordCell, ForgetClosedFiles, usePasswordSet } from "../../components/PasswordFields";
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
  const passwords = usePasswordSet();
  const navigation: Pick<ReportActions, "canOpen" | "open"> = {
    canOpen: (target) => Boolean(onNavigate) && availability.allows(target),
    open: (target, path) => onNavigate?.({ utilityId: target, initialPaths: [path] }),
  };
  const report = tool.report;

  useEffect(() => {
    passwords.clear();
  }, [utility.id]);

  return (
    <ToolScaffold
      variant="workspace"
      sessionKey="pdf-editor-batch"
      utility={utility}
      initialPaths={initialPaths}
      onFilesChange={onFilesChange}
      onRun={(paths, outputLocation, { runId }) => invoke<ToolResult>(utility.command, { runId, request: tool.request(paths, outputLocation, passwords.set) })}
      retryWithPassword={tool.openPassword ? (inputPath, password) => (paths, outputLocation, { runId }) =>
        invoke<ToolResult>(utility.command, { runId, request: tool.request(paths, outputLocation, { batch: "", overrides: { [inputPath]: password } }) }) : undefined}
      renderFileExtra={tool.openPassword ? (path, selected, locked) => (
        <FilePasswordCell mode="open" path={path} passwords={passwords} selected={selected} disabled={locked} />
      ) : undefined}
      renderReport={report && ((results, { withPassword, ...run }) => report.render(results, { ...navigation, ...run, retryWithPassword: withPassword }))}
      destinationCopy={report?.destination}
    >
      {({ files, run, loading, destination, runLabel }) => (
        <Card className="workspace-control-panel py-0">
          {tool.openPassword && <ForgetClosedFiles files={files} passwords={passwords} />}
          <CardContent className="workspace-control-panel-content grid gap-4 p-4">
            <div>
              <h2 className="workspace-active-command">{utility.title}</h2>
              <p className="workspace-panel-copy">{tool.description}</p>
            </div>
            {tool.openPassword && <BatchPasswordFields mode="open" passwords={passwords} files={files} />}
            {destination}
            {tool.report && <p className="workspace-note">{tool.report.note}</p>}
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
