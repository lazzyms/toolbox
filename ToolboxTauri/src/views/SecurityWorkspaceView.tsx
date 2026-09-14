import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import { WorkspaceCommandRail } from "../components/WorkspaceCommandRail";
import { toolsForWorkspaceId, UtilityRegistry } from "../registry";
import type { AtomicToolId, PasswordRequest, PDFRequest, ToolDefinition, ToolResult } from "../contracts";

const securityActions = toolsForWorkspaceId("file-security");
const securityIds = new Set<string>(securityActions.map((tool) => tool.id));

export const SecurityWorkspaceView = ({ utility }: { utility: ToolDefinition }) => {
  const [password, setPassword] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [activeToolId, setActiveToolId] = useState<AtomicToolId>(
    securityIds.has(utility.id) ? utility.id : "pdf-unlock",
  );
  useEffect(() => {
    setActiveToolId(securityIds.has(utility.id) ? utility.id : "pdf-unlock");
  }, [utility.id]);
  const activeUtility = securityIds.has(utility.id)
    ? UtilityRegistry.find((item) => item.id === activeToolId) ?? utility
    : UtilityRegistry.find((item) => item.id === activeToolId) ?? utility;
  const isPdfProtection = activeUtility.id === "pdf-protect";
  const isOfficeProtection = activeUtility.id === "office-protect";

  useEffect(() => {
    setPassword("");
    setShowPassword(false);
  }, [activeUtility.id]);

  return (
    <ToolScaffold
      variant="workspace"
      sessionKey="file-security"
      utility={activeUtility}
      onRun={(paths) =>
        isPdfProtection
          ? invoke<ToolResult>("protect_pdf", {
              request: {
                paths,
                password,
                outputLocation: "alongsideInput",
              } satisfies PDFRequest,
            })
          : isOfficeProtection
            ? invoke<ToolResult>("protect_office", {
                request: {
                  paths,
                  password,
                  outputLocation: "alongsideInput",
                } satisfies PasswordRequest,
              })
          : invoke<ToolResult>("remove_password", {
              request: {
                paths,
                password,
                outputLocation: "alongsideInput",
              } satisfies PasswordRequest,
            })
      }
    >
      {({ files, run, loading }) => (
        <div className="workspace-control-panel">
          <WorkspaceCommandRail
            actions={securityActions}
            activeId={activeToolId}
            onSelect={setActiveToolId}
            label="File security tools"
          />
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
          <label className="workspace-field">
            <span>{isPdfProtection || isOfficeProtection ? "New password" : "Current password"}</span>
            <span className="workspace-password-field">
              <input
                aria-label={isPdfProtection || isOfficeProtection ? "New password" : "Current password"}
                type={showPassword ? "text" : "password"}
                value={password}
                onChange={(event) => setPassword(event.target.value)}
                placeholder="Enter password"
                autoComplete="off"
              />
              <button
                type="button"
                className="workspace-inline-button"
                aria-pressed={showPassword}
                onClick={() => setShowPassword((visible) => !visible)}
              >
                {showPassword ? "Hide" : "Show"}
              </button>
            </span>
          </label>
          <button
            type="button"
            disabled={loading || files.length === 0 || password.length === 0}
            onClick={run}
            className="workspace-primary-action"
          >
            {activeUtility.shortTitle}
          </button>
        </div>
      )}
    </ToolScaffold>
  );
};
