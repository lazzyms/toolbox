import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold, type RunHandler } from "../components/ToolScaffold";
import { BatchPasswordFields, FilePasswordCell, passwordReadiness, passwordRequest, usePasswordSet, type PasswordMode, type PasswordState } from "../components/PasswordFields";
import type { AtomicToolId, PasswordSet, ToolCommand, ToolDefinition, ToolResult } from "../contracts";
import { capabilityFor } from "../registry/capabilities";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { ShortcutHint } from "../components/ShortcutHint";
import { shortcutTarget } from "../shortcuts";

const commandFor = (toolId: AtomicToolId, mode: PasswordMode): ToolCommand =>
  toolId === "office-protect" ? "protect_office" : mode === "lock" ? "protect_files" : "remove_password";

const runWith = (command: ToolCommand, passwords: PasswordSet): RunHandler => (paths, outputLocation, { runId }) =>
  invoke<ToolResult>(command, { runId, request: passwordRequest(paths, passwords, outputLocation) });

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
  const isOfficeProtection = activeUtility.id === "office-protect";
  const isLockUnlock = activeUtility.id === "lock-unlock";
  const mode: PasswordMode = isLockUnlock ? chosenMode : "lock";
  const command = commandFor(activeUtility.id, mode);
  const lockingWithLockUnlock = isLockUnlock && mode === "lock";
  const utility = useMemo<ToolDefinition>(() => lockingWithLockUnlock
    ? {
      ...activeUtility,
      resultVerb: "Locked",
      runningVerb: "Locking",
      capability: { ...activeUtility.capability, outputSuffix: capabilityFor("protect_files").outputSuffix },
    }
    : activeUtility, [activeUtility, lockingWithLockUnlock]);

  useEffect(() => {
    passwords.clear();
  }, [activeUtility.id, mode]);

  return (
    <ToolScaffold
      variant="workspace"
      sessionKey="file-security"
      utility={utility}
      onRun={runWith(command, passwords.set)}
      retryWithPassword={(password) => runWith(command, { batch: password, overrides: {} })}
      renderFileExtra={(path, selected, locked) => (
        <FilePasswordCell mode={mode} path={path} passwords={passwords} selected={selected} disabled={locked} />
      )}
    >
      {({ files, run, loading, destination }) => {
        const readiness = passwordReadiness(mode, passwords, files);
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
              {isOfficeProtection
                ? "Add a password to selected DOCX and XLSX files. The originals stay untouched."
                : mode === "lock"
                  ? "Each PDF gets a copy ending in -protected that opens only with the new password. Your originals stay as they are."
                  : "Each file gets a copy ending in -unlocked that opens without a password. Your originals stay as they are."}
            </p>
            <p className="workspace-note">
              {isOfficeProtection
                ? "DOCX and XLSX files only"
                : mode === "lock"
                  ? "Accepts .pdf"
                  : "Accepts .pdf, .doc, .docx, .xls, .xlsx, .ppt, .pptx"}
            </p>
          </div>
          {isLockUnlock && mode === "lock" && (
            <div className="grid gap-2">
              <p className="workspace-note">To lock Word and Excel files, use Protect Office Files.</p>
              <Button variant="outline" size="sm" onClick={() => openTool("office-protect")}>Open Protect Office Files</Button>
            </div>
          )}
          <BatchPasswordFields mode={mode} passwords={passwords} files={files} />
          {destination}
          {isLockUnlock && <p className="workspace-note">An existing name gets -1, -2 and so on. Nothing is replaced.</p>}
          {files.length > 0 && readiness.reason && <p className="workspace-note" data-testid="password-readiness">{readiness.reason}</p>}
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
