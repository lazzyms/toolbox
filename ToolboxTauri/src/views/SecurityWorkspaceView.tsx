import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import { WorkspaceCommandRail } from "../components/WorkspaceCommandRail";
import { toolsForWorkspaceId, UtilityRegistry } from "../registry";
import type { AtomicToolId, PasswordRequest, PDFRequest, ToolDefinition, ToolResult } from "../contracts";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Card, CardContent } from "@/components/ui/card";

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
        <Card className="workspace-control-panel py-0">
          <CardContent className="workspace-control-panel-content grid gap-4 p-4">
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
          <div className="workspace-field">
            <Label htmlFor="security-password">{isPdfProtection || isOfficeProtection ? "New password" : "Current password"}</Label>
            <div className="workspace-password-field">
              <Input
                id="security-password"
                type={showPassword ? "text" : "password"}
                value={password}
                onChange={(event) => setPassword(event.target.value)}
                placeholder="Enter password"
                autoComplete="off"
              />
              <Button
                variant="ghost"
                size="sm"
                className="workspace-inline-button"
                aria-label={`${showPassword ? "Hide" : "Show"} ${isPdfProtection || isOfficeProtection ? "new" : "current"} password`}
                onClick={() => setShowPassword((visible) => !visible)}
              >
                {showPassword ? "Hide" : "Show"}
              </Button>
            </div>
          </div>
          <Button
            variant="default"
            aria-label={activeUtility.id === "pdf-unlock" ? "Remove Password from selected files" : activeUtility.shortTitle}
            disabled={loading || files.length === 0 || password.length === 0}
            onClick={run}
            className="workspace-primary-action"
          >
            {activeUtility.shortTitle}
          </Button>
          </CardContent>
        </Card>
      )}
    </ToolScaffold>
  );
};
