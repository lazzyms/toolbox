import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold, type RunHandler } from "../components/ToolScaffold";
import { BatchPasswordFields, FilePasswordCell, passwordReadiness, passwordRequest, usePasswordSet, type PasswordMode, type PasswordState } from "../components/PasswordFields";
import type { PasswordSet, ToolCommand, ToolDefinition, ToolResult } from "../contracts";
import { capabilityFor } from "../registry/capabilities";
import { extensionOf, type IntakeCopy } from "../components/intake";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { ShortcutHint } from "../components/ShortcutHint";
import { shortcutTarget } from "../shortcuts";

const commandFor = (mode: PasswordMode): ToolCommand => mode === "lock" ? "protect_files" : "remove_password";

const runWith = (command: ToolCommand, passwords: PasswordSet): RunHandler => (paths, outputLocation, { runId }) =>
  invoke<ToolResult>(command, { runId, request: passwordRequest(paths, passwords, outputLocation) });

// Closing a file drops its own password, and closing every file drops them all.
const ForgetClosedFiles = ({ files, passwords }: { files: string[]; passwords: PasswordState }) => {
  useEffect(() => {
    passwords.keepOnly(files);
  }, [files]);
  return null;
};

const fileCount = (count: number) => `${count} ${count === 1 ? "file" : "files"}`;

const NOT_A_DOCUMENT = "Not a document. Lock and Unlock take PDF, Word, Excel and PowerPoint files.";

const modernFormat: Record<string, { extension: string; app: string }> = {
  ".doc": { extension: ".docx", app: "Word" },
  ".xls": { extension: ".xlsx", app: "Excel" },
  ".ppt": { extension: ".pptx", app: "PowerPoint" },
};

const intakeCopy: Record<PasswordMode, IntakeCopy> = {
  lock: {
    notAccepted: NOT_A_DOCUMENT,
    unsupported: (path) => {
      const modern = modernFormat[extensionOf(path)];
      return modern ? `Lock needs ${modern.extension}. Save it as ${modern.extension} in ${modern.app}, then add it again.` : NOT_A_DOCUMENT;
    },
    flaggedNotice: (count) => `${fileCount(count)} can't be locked. Lock needs .docx, .xlsx or .pptx. ${count === 1 ? "It'd" : "They'd"} fail if you ran now.`,
    countHint: (flagged, total) => flagged > 0 ? `${flagged} won't run in Lock` : total === 1 ? "It can be locked" : `All ${total} can be locked`,
  },
  unlock: {
    notAccepted: NOT_A_DOCUMENT,
    unsupported: () => NOT_A_DOCUMENT,
    flaggedNotice: (count) => `${fileCount(count)} can't be unlocked. ${count === 1 ? "It'd" : "They'd"} fail if you ran now.`,
    countHint: (flagged, total) => flagged > 0 ? `${flagged} won't run in Unlock` : total === 1 ? "It can be unlocked" : `All ${total} can be unlocked`,
  },
};

export const SecurityWorkspaceView = ({ utility: activeUtility }: { utility: ToolDefinition }) => {
  const passwords = usePasswordSet();
  const [mode, setMode] = useState<PasswordMode>("unlock");
  const command = commandFor(mode);
  const utility = useMemo<ToolDefinition>(() => mode === "lock"
    ? {
      ...activeUtility,
      resultVerb: "Locked",
      runningVerb: "Locking",
      capability: capabilityFor("protect_files"),
    }
    : activeUtility, [activeUtility, mode]);

  useEffect(() => {
    passwords.clear();
  }, [activeUtility.id, mode]);

  return (
    <ToolScaffold
      variant="workspace"
      sessionKey="file-security"
      utility={utility}
      intakeCopy={intakeCopy[mode]}
      onRun={runWith(command, passwords.set)}
      retryWithPassword={(inputPath, password) => runWith(command, { batch: "", overrides: { [inputPath]: password } })}
      renderFileExtra={(path, selected, locked) => (
        <FilePasswordCell mode={mode} path={path} passwords={passwords} selected={selected} disabled={locked} />
      )}
    >
      {({ files, run, loading, destination, failedFiles, retryFailed, retrying }) => {
        const readiness = passwordReadiness(mode, passwords, files);
        const retryReadiness = passwordReadiness(mode, passwords, failedFiles);
        const retryLabel = `Retry ${failedFiles.length} failed ${failedFiles.length === 1 ? "file" : "files"}`;
        const verb = mode === "lock" ? "Lock" : "Unlock";
        const runLabel = files.length === 0 ? `${verb} files` : `${verb} ${fileCount(files.length)}`;
        return (
        <Card className="workspace-control-panel py-0">
          <ForgetClosedFiles files={files} passwords={passwords} />
          <CardContent className="workspace-control-panel-content grid gap-4 p-4">
          <div>
            <h2 className="workspace-active-command">{activeUtility.title}</h2>
            <ToggleGroup
              type="single"
              variant="outline"
              size="sm"
              aria-label="Lock or unlock"
              value={mode}
              onValueChange={(value) => {
                if (value) setMode(value as PasswordMode);
              }}
            >
              <ToggleGroupItem value="lock">Lock</ToggleGroupItem>
              <ToggleGroupItem value="unlock">Unlock</ToggleGroupItem>
            </ToggleGroup>
            <p className="workspace-panel-copy">
              {mode === "lock"
                ? "Each file gets a copy ending in -protected that opens only with the new password. Your originals stay as they are."
                : "Each file gets a copy ending in -unlocked that opens without a password. Your originals stay as they are."}
            </p>
            <p className="workspace-note">
              {mode === "lock" ? "Accepts .pdf, .docx, .xlsx, .pptx" : "Accepts .pdf, .doc, .docx, .xls, .xlsx, .ppt, .pptx"}
            </p>
          </div>
          <BatchPasswordFields mode={mode} passwords={passwords} files={files} />
          {destination}
          <p className="workspace-note">An existing name gets -1, -2 and so on. Nothing is replaced.</p>
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
          {failedFiles.length > 0 && (
            <Button
              variant="outline"
              disabled={loading || retrying || !retryReadiness.ready}
              onClick={() => void retryFailed()}
            >
              {retrying ? "Retrying..." : retryLabel}
            </Button>
          )}
          </CardContent>
        </Card>
        );
      }}
    </ToolScaffold>
  );
};
