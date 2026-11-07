import { useState } from "react";
import ReactDOM from "react-dom/client";
import { WorkspaceCommandRail } from "../../../src/components/WorkspaceCommandRail";
import { UtilityRegistry, toolsForWorkspace, workspaceById } from "../../../src/registry";
import { ToolAvailabilityProvider } from "../../../src/ToolAvailabilityContext";
import { OutputLocationContext } from "../../../src/outputLocation";
import { PDFEditorWorkspaceView } from "../../../src/views/PDFEditorWorkspaceView";
import { pdfEditorBatchTools, type PdfEditorBatchTool, type PdfEditorBatchTools } from "../../../src/features/pdf-editor/batchTools";
import type { AtomicToolId, OutputLocation, ToolCommand, ToolDefinition, ToolWorkspaceDefinition } from "../../../src/contracts";
import type { ToolAvailabilitySnapshot } from "../../../src/toolFlags";
import "../../../src/styles/app.css";

// Besides the shipped batch tools, the harness adds an export tool that exists only in tests.
const testToolId = "pdf-test-batch" as AtomicToolId;
const testTool: ToolDefinition = {
  id: testToolId,
  title: "Test Batch Tool",
  shortTitle: "Test Batch",
  resultVerb: "Tested",
  runningVerb: "Testing",
  blurb: "A batch tool that exists only in tests.",
  symbol: "files",
  category: "PDF",
  command: "test_batch_tool" as ToolCommand,
  verification: "pdf-test-batch",
  view: "pdf-test-batch" as ToolDefinition["view"],
  status: "implemented",
  capability: {
    acceptedExtensions: [".pdf"],
    inputCardinality: "multiple",
    supportsPageSelection: false,
    supportsPreview: false,
    nativeAvailability: "available",
    outputSuffix: "-tested",
    manyToOne: false,
  },
};
const testBatchTool: PdfEditorBatchTool = {
  description: "Runs the test command on every PDF.",
  request: (paths, outputLocation) => ({ paths, outputLocation, mode: "test" }),
};
const batchTools: PdfEditorBatchTools = { ...pdfEditorBatchTools, [testToolId]: testBatchTool };

const pdfEditor = workspaceById("pdf-editor")!;
const workspace: ToolWorkspaceDefinition = { ...pdfEditor, toolIds: [...pdfEditor.toolIds, testToolId] };
const tools = [...toolsForWorkspace(pdfEditor), testTool];

// ?tool= opens another rail tool first, ?folder= saves to a custom folder instead of next to each file,
// and ?disabled= turns a tool off.
const query = new URLSearchParams(window.location.search);
const availability = {
  platform: "macos",
  enabledByTool: Object.fromEntries([...UtilityRegistry, testTool].map((tool) => [tool.id, !query.getAll("disabled").includes(tool.id)])),
} as ToolAvailabilitySnapshot;

const firstTool = tools.find((tool) => tool.id === query.get("tool")) ?? testTool;
const folder = query.get("folder");
const outputLocation: OutputLocation = folder ? { customFolder: folder } : "alongsideInput";

// Like the main page, a rail switch reopens the new tool with the files that were open, and a
// report pointer opens its tool with that report's file. Pointer navigations are recorded for tests.
const navigations: unknown[] = ((window as unknown as { navigations: unknown[] }).navigations = []);
const Harness = () => {
  const [active, setActive] = useState({ tool: firstTool, paths: [] as readonly string[] });
  const [openPaths, setOpenPaths] = useState<readonly string[]>([]);
  return (
    <div className="app-shell app-shell--view">
      <aside className="app-rail" aria-label="PDF Editor">
        <WorkspaceCommandRail
          actions={tools}
          workspace={workspace}
          activeId={active.tool.id}
          onSelect={(id) => setActive({ tool: tools.find((tool) => tool.id === id)!, paths: openPaths })}
          label="PDF Editor tools"
        />
      </aside>
      <main className="command-center" aria-label="Tool detail">
        <PDFEditorWorkspaceView
          key={active.tool.id}
          utility={active.tool}
          initialPaths={active.paths}
          onFilesChange={setOpenPaths}
          onNavigate={(navigation) => {
            navigations.push(navigation);
            setActive({ tool: tools.find((tool) => tool.id === navigation.utilityId)!, paths: navigation.initialPaths });
          }}
          batchTools={batchTools}
        />
      </main>
    </div>
  );
};

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <ToolAvailabilityProvider snapshot={availability}>
    <OutputLocationContext.Provider value={outputLocation}>
      <Harness />
    </OutputLocationContext.Provider>
  </ToolAvailabilityProvider>,
);
