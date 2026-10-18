import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold, type RunHandler } from "../components/ToolScaffold";
import { BatchPasswordFields, FilePasswordCell, passwordReadiness, passwordRequest, usePasswordSet, type PasswordMode, type PasswordState } from "../components/PasswordFields";
import type { AtomicToolId, PasswordSet, ToolDefinition, ToolResult } from "../contracts";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { ShortcutHint } from "../components/ShortcutHint";
import { shortcutTarget } from "../shortcuts";

const commands: Partial<Record<AtomicToolId, string>> = {
  "pdf-protect": "protect_pdf",
  "office-protect": "protect_office",
};

const runWith = (toolId: AtomicToolId, passwords: PasswordSet): RunHandler => (paths, outputLocation, { runId }) =>
  invoke<ToolResult>(commands[toolId] ?? "remove_password", { runId, request: passwordRequest(paths, passwords, outputLocation) });

// Closing a file drops its own password, and closing every file drops them all.
const ForgetClosedFiles = ({ files, passwords }: { files: string[]; passwords: PasswordState }) => {
  useEffect(() => {
    passwords.keepOnly(files);
  }, [files]);
  return null;
};

const openTool = (id: AtomicToolId) => window.dispatchEvent(new CustomEvent("toolbox:open", { detail: id }));

const fileCount = (count: number) => `${count} ${count === 1 ? "file" : "files"}`;

export const SecurityWorkspaceView = ({ utility: activeUtility }: { utility: ToolDefinition }) => {
  const passwords = usePasswordSet();
  const [chosenMode, setChosenMode] = useState<PasswordMode>("unlock");
  const isPdfProtection = activeUtility.id === "pdf-protect";
  const isOfficeProtection = activeUtility.id === "office-protect";
  const isLockUnlock = activeUtility.id === "lock-unlock";
  const mode: PasswordMode = isLockUnlock ? chosenMode : "lock";
  // Lock inside Lock & Unlock is not wired to a command yet, so Protect PDF and Protect Office Files keep that job.
  const lockPending = isLockUnlock && mode === "lock";

  useEffect(() => {
    passwords.clear();
  }, [activeUtility.id, mode]);

  return (
    <ToolScaffold
      variant="workspace"
      sessionKey="file-security"
      utility={activeUtility}
      onRun={runWith(activeUtility.id, passwords.set)}
      retryWithPassword={(password) => runWith(activeUtility.id, { batch: password, overrides: {} })}
      renderFileExtra={(path, selected, locked) => (
        <FilePasswordCell mode={mode} path={path} passwords={passwords} selected={selected} disabled={locked} />
      )}
    >
      {({ files, run, loading, destination }) => {
        const readiness = lockPending
          ? { ready: false, reason: "To lock files for now, use Protect PDF or Protect Office Files." }
          : passwordReadiness(mode, passwords, files);
        const verb = mode === "lock" ? "Lock" : "Unlock";
        const runLabel = isLockUnlock ? (files.length === 0 ? `${verb} files` : `${verb} ${fileCount(files.length)}`) : activeUtility.shortTitle;
        return (
        <Card className="workspace-control-panel py-0">
          <ForgetClosedFiles files={files} passwords={passwords} />
          <CardContent className="workspace-control-panel-content grid gap-4 p-4">
          <div>
            <h2 className="workspace-active-command">{activeUtility.title}</h2>
            {isLockUnlock && (
              <ToggleGroup
                type="single"
                variant="outline"
                size="sm"
                aria-label="Lock or unlock"
                value={mode}
                onValueChange={(value) => {
                  if (value) setChosenMode(value as PasswordMode);
                }}
              >
                <ToggleGroupItem value="lock">Lock</ToggleGroupItem>
                <ToggleGroupItem value="unlock">Unlock</ToggleGroupItem>
              </ToggleGroup>
            )}
            <p className="workspace-panel-copy">
              {isPdfProtection
                ? "Add a password to each selected PDF. The originals stay untouched."
                : isOfficeProtection
                  ? "Add a password to selected DOCX and XLSX files. The originals stay untouched."
                  : lockPending
                    ? "Add a password to PDF, Word and Excel files."
                    : "Each file gets a copy ending in -unlocked that opens without a password. Your originals stay as they are."}
            </p>
            <p className="workspace-note">
              {isPdfProtection
                ? "PDF files only"
                : isOfficeProtection
                  ? "DOCX and XLSX files only"
                  : "Accepts .pdf, .doc, .docx, .xls, .xlsx, .ppt, .pptx"}
            </p>
          </div>
          {lockPending ? (
            <div className="grid gap-2">
              <Button variant="outline" size="sm" onClick={() => openTool("pdf-protect")}>Open Protect PDF</Button>
              <Button variant="outline" size="sm" onClick={() => openTool("office-protect")}>Open Protect Office Files</Button>
            </div>
          ) : (
            <>
              <BatchPasswordFields mode={mode} passwords={passwords} files={files} />
              {destination}
              {isLockUnlock && <p className="workspace-note">An existing name gets -1, -2 and so on. Nothing is replaced.</p>}
            </>
          )}
          {(files.length > 0 || lockPending) && readiness.reason && <p className="workspace-note" data-testid="password-readiness">{readiness.reason}</p>}
          <Button
            variant="default"
            aria-label={runLabel}
            disabled={loading || files.length === 0 || !readiness.ready}
            onClick={run}
            className="workspace-primary-action"
            {...shortcutTarget("run")}
          >
            {runLabel}
            <ShortcutHint id="run" />
          </Button>
          </CardContent>
        </Card>
        );
      }}
    </ToolScaffold>
  );
};
