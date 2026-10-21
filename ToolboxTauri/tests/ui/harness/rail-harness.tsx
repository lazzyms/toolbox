import { useState } from "react";
import ReactDOM from "react-dom/client";
import { WorkspaceCommandRail } from "../../../src/components/WorkspaceCommandRail";
import { UtilityRegistry } from "../../../src/registry";
import { ToolAvailabilityProvider } from "../../../src/ToolAvailabilityContext";
import type { ToolAvailabilitySnapshot } from "../../../src/toolFlags";
import "../../../src/styles/app.css";

// No real view holds ten or more tools yet, so the rail is exercised with twelve registered ones.
const tools = UtilityRegistry.slice(0, 12);

const everyToolEnabled = {
  platform: "macos",
  enabledByTool: Object.fromEntries(UtilityRegistry.map((tool) => [tool.id, true])),
} as ToolAvailabilitySnapshot;

const Harness = () => {
  const [activeId, setActiveId] = useState(tools[0].id);
  return (
    <div className="app-shell app-shell--view">
      <aside className="app-rail" aria-label="Rail harness">
        <WorkspaceCommandRail actions={tools} activeId={activeId} onSelect={setActiveId} label="Harness tools" />
      </aside>
      <main className="command-center" aria-label="Tool detail">
        <p data-testid="active-tool">{activeId}</p>
      </main>
    </div>
  );
};

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <ToolAvailabilityProvider snapshot={everyToolEnabled}>
    <Harness />
  </ToolAvailabilityProvider>,
);
