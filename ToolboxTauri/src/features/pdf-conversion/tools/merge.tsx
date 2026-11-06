import { invoke } from "@tauri-apps/api/core";
import type { ToolResult } from "../../../contracts";
import type { ConversionTool } from "../tool";

export const merge: ConversionTool = {
  inspectsPdf: false,
  outputPreview: () => "Output preview unavailable. Export merges the selected PDFs in file order.",
  exportBlocked: (_options, files) => files.length <= 1,
  OptionsPanel: () => <p className="workspace-note">Add at least two PDFs. The order in the source list becomes the page order in the merged copy.</p>,
  request: (paths, outputLocation, { outputPath, runId }) => invoke<ToolResult>("merge_pdfs", { runId,
    request: { paths, outputLocation, outputPath },
  }),
};
