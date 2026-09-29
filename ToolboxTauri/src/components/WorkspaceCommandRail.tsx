import type { KeyboardEvent } from "react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { AtomicToolId, ToolDefinition } from "../contracts";
import { TablerIcon } from "./TablerIcon";

interface WorkspaceCommandRailProps {
  actions: ToolDefinition[];
  activeId: AtomicToolId;
  onSelect: (id: AtomicToolId) => void;
  label: string;
}

const handleToolbarKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
  if (event.altKey || event.ctrlKey || event.metaKey) return;
  const buttons = Array.from(
    event.currentTarget.querySelectorAll<HTMLButtonElement>("button:not(:disabled)"),
  );
  const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
  if (index < 0 || buttons.length === 0) return;
  const nextIndex =
    event.key === "ArrowDown" ? (index + 1) % buttons.length
      : event.key === "ArrowUp" ? (index - 1 + buttons.length) % buttons.length
        : event.key === "Home" ? 0
          : event.key === "End" ? buttons.length - 1
            : -1;
  if (nextIndex < 0) return;
  event.preventDefault();
  buttons.forEach((button, buttonIndex) => {
    button.tabIndex = buttonIndex === nextIndex ? 0 : -1;
  });
  buttons[nextIndex].focus();
};

/**
 * Persistent command palette for a workspace. Commands change the inspector
 * context while the open document and its selection stay in place. This is
 * intentionally a navigation rail, not a tab list.
 */
export const WorkspaceCommandRail = ({
  actions,
  activeId,
  onSelect,
  label,
}: WorkspaceCommandRailProps) => (
  <Card className="workspace-command-rail py-0" role="complementary" aria-label={label}>
    <CardHeader className="workspace-command-rail-header px-4 py-3">
      <CardTitle className="workspace-command-rail-label">Tools</CardTitle>
    </CardHeader>
    <CardContent
      className="workspace-command-list p-2"
      role="toolbar"
      aria-label={label}
      aria-orientation="vertical"
      onKeyDown={handleToolbarKeyDown}
    >
      {actions.map((tool) => {
        const unavailable = tool.capability.nativeAvailability === "unavailable";
        return (
          <Button
            variant={activeId === tool.id ? "secondary" : "ghost"}
            size="sm"
            key={tool.id}
            type="button"
            className="workspace-command"
            aria-pressed={activeId === tool.id}
            tabIndex={activeId === tool.id ? 0 : -1}
            aria-label={tool.title}
            title={unavailable ? `${tool.title} is unavailable in this build` : tool.blurb}
            disabled={unavailable}
            onClick={() => onSelect(tool.id)}
          >
            <span className="workspace-command-icon" aria-hidden="true">
              <TablerIcon name={tool.symbol} />
            </span>
            <span className="workspace-command-copy">
              <strong>{tool.shortTitle}</strong>
              {unavailable && <small>Unavailable</small>}
            </span>
          </Button>
        );
      })}
    </CardContent>
  </Card>
);
