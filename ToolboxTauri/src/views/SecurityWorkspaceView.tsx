import { useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold, type RunHandler } from "../components/ToolScaffold";
import { BatchPasswordFields, FilePasswordCell, passwordReadiness, passwordRequest, usePasswordSet, type PasswordMode, type PasswordState } from "../components/PasswordFields";
import type { AtomicToolId, PasswordSet, ToolDefinition, ToolResult } from "../contracts";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
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

export const SecurityWorkspaceView = ({ utility: activeUtility }: { utility: ToolDefinition }) => {
  const passwords = usePasswordSet();
  const isPdfProtection = activeUtility.id === "pdf-protect";
  const isOfficeProtection = activeUtility.id === "office-protect";
  const mode: PasswordMode = isPdfProtection || isOfficeProtection ? "lock" : "unlock";

  useEffect(() => {
    passwords.clear();
  }, [activeUtility.id]);

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
        const readiness = passwordReadiness(mode, passwords, files);
        return (
        <Card className="workspace-control-panel py-0">
          <ForgetClosedFiles files={files} passwords={passwords} />
          <CardContent className="workspace-control-panel-content grid gap-4 p-4">
          <div>
            <h2 className="workspace-active-command">{activeUtility.title}</h2>
            <p className="workspace-panel-label">
              {isPdfProtection || isOfficeProtection ? "Protect a file" : "Unlock a file"}
            </p>
            <p className="workspace-panel-copy">
              {isPdfProtection
                ? "Add a password to each selected PDF. The originals stay untouched."
                : isOfficeProtection
                  ? "Add a password to selected DOCX and XLSX files. The originals stay untouched."
                : "Use the existing password to save an unlocked copy of each selected PDF or Office file."}
            </p>
            <p className="workspace-note">
              {isPdfProtection
                ? "PDF files only"
                : isOfficeProtection
                  ? "DOCX and XLSX files only"
                : "PDF, Word, Excel, and PowerPoint files"}
            </p>
          </div>
          <BatchPasswordFields mode={mode} passwords={passwords} files={files} />
          {destination}
          {files.length > 0 && readiness.reason && <p className="workspace-note" data-testid="password-readiness">{readiness.reason}</p>}
          <Button
            variant="default"
            aria-label={activeUtility.id === "pdf-unlock" ? "Remove Password from selected files" : activeUtility.shortTitle}
            disabled={loading || files.length === 0 || !readiness.ready}
            onClick={run}
            className="workspace-primary-action"
            {...shortcutTarget("run")}
          >
            {activeUtility.shortTitle}
            <ShortcutHint id="run" />
          </Button>
          </CardContent>
        </Card>
        );
      }}
    </ToolScaffold>
  );
};
