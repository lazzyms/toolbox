import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import { UtilityRegistry } from "../registry";
import type { PasswordRequest, PDFRequest, ToolDefinition, ToolResult } from "../contracts";

const securityIds = new Set(["pdf-unlock", "pdf-protect"]);

export const SecurityWorkspaceView = ({ utility }: { utility: ToolDefinition }) => {
  const [password, setPassword] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const activeUtility = securityIds.has(utility.id)
    ? utility
    : UtilityRegistry.find((item) => item.id === "pdf-unlock") ?? utility;

  return (
    <ToolScaffold
      utility={activeUtility}
      onRun={(paths) =>
        activeUtility.id === "pdf-protect"
          ? invoke<ToolResult>("protect_pdf", {
              request: {
                paths,
                password,
                outputLocation: "alongsideInput",
              } satisfies PDFRequest,
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
          <div>
            <p className="workspace-panel-label">
              {activeUtility.id === "pdf-protect" ? "Protect a file" : "Unlock a file"}
            </p>
            <p className="workspace-panel-copy">
              {activeUtility.id === "pdf-protect"
                ? "Add a password to each selected PDF. The originals stay untouched."
                : "Use the existing password to save an unlocked copy of each selected PDF or Office file."}
            </p>
          </div>
          <label className="workspace-field">
            <span>{activeUtility.id === "pdf-protect" ? "New password" : "Current password"}</span>
            <span className="workspace-password-field">
              <input
                aria-label={activeUtility.id === "pdf-protect" ? "New password" : "Current password"}
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
