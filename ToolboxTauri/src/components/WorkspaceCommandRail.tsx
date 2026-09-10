import type { AtomicToolId, ToolDefinition } from "../contracts";
import { TablerIcon } from "./TablerIcon";

interface WorkspaceCommandRailProps {
  actions: ToolDefinition[];
  activeId: AtomicToolId;
  onSelect: (id: AtomicToolId) => void;
  label: string;
}

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
  <aside className="workspace-command-rail" aria-label={label}>
    <p className="workspace-command-rail-label">Tools</p>
    <nav role="toolbar" aria-label={label}>
      {actions.map((tool) => {
        const unavailable = tool.capability.nativeAvailability === "unavailable";
        return (
          <button
            key={tool.id}
            type="button"
            className="workspace-command"
            data-active={activeId === tool.id ? "true" : undefined}
            aria-pressed={activeId === tool.id}
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
          </button>
        );
      })}
    </nav>
  </aside>
);
