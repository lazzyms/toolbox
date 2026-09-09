import type { ToolDefinition } from "../contracts";

interface WorkspaceActionBarProps {
  actions: ToolDefinition[];
  activeId: string;
  onSelect: (tool: ToolDefinition) => void;
  label: string;
}

export const WorkspaceActionBar = ({
  actions,
  activeId,
  onSelect,
  label,
}: WorkspaceActionBarProps) => (
  <div className="workspace-action-bar" role="tablist" aria-label={label}>
    {actions.map((tool) => (
      <button
        key={tool.id}
        type="button"
        role="tab"
        aria-selected={activeId === tool.id}
        aria-label={`Open ${tool.title}`}
        onClick={() => onSelect(tool)}
      >
        {tool.shortTitle}
      </button>
    ))}
  </div>
);
