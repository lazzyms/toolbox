import { invoke } from "@tauri-apps/api/core";
import type { ToolResult } from "../../../contracts";
import type { ConversionTool } from "../tool";

const fileName = (path: string) => path.split(/[\\/]/).pop() ?? path;

export const compare: ConversionTool = {
  inspectsPdf: false,
  outputPreview: () => "Output preview unavailable. Export creates a text file listing every line that changed, page by page.",
  exportBlocked: (_options, files) => files.length !== 2,
  OptionsPanel: ({ files }) => files.length === 2 ? (
    <div className="grid gap-2" aria-live="polite">
      <p className="workspace-note">Original: <strong>{fileName(files[0])}</strong></p>
      <p className="workspace-note">New version: <strong>{fileName(files[1])}</strong></p>
      <p className="workspace-note">Drag the files in the list to swap them.</p>
    </div>
  ) : (
    <p className="workspace-note" role="status">Choose exactly two PDFs: the original and the new version.</p>
  ),
  request: (paths, outputLocation, { outputPath, runId }) => invoke<ToolResult>("compare_pdfs", { runId,
    request: { paths, outputLocation, outputPath },
  }),
};
